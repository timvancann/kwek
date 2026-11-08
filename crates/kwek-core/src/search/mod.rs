//! Search over conversation stretches (see [`chunk`]): keyword relevance over each stretch
//! as a whole, so words spread over a few messages still find the conversation, merged
//! with closeness in meaning when the embedding model is available.

pub mod chunk;
pub mod embed;

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use crate::db::read::{self, SearchFilter};
use crate::error::Result;
use crate::ts::{self, Micros};
use crate::views::{Builder, SearchHit, SearchLine, SnippetPart, snippet_parts};

/// Reciprocal rank fusion constant: the usual 60 flattens the head less than we want for a
/// list of at most a few dozen hits.
const RRF_K: f32 = 20.0;
/// Meaning candidates considered. Few: a small model's tail is mostly noise.
const MEANING_TOP: usize = 10;
/// Meaning hits must be this close to the best one: e5 similarities sit in a narrow band
/// (on real chats the best is ~0.85 and the median ~0.78), so only the head is signal.
const MEANING_BAND: f32 = 0.025;
/// The best this many all-words matches rank above every related result; all-words matches
/// beyond them are fused with related results like any other keyword hit, so a common word
/// still leaves room for related results on the first screen.
const FULL_LEAD: usize = 15;
const LINES: usize = 3;
const PLAIN_LINE: usize = 160;

#[derive(Debug, Clone, Default)]
pub struct Query {
    /// Words as typed, FTS syntax removed.
    pub terms: Vec<String>,
    pub filter: SearchFilter,
}

impl Query {
    pub fn has_filter(&self) -> bool {
        let f = &self.filter;
        f.from.is_some() || f.space.is_some() || f.before_us.is_some() || f.after_us.is_some()
    }

    /// Whether a message satisfies the filters on sender and date, the same test as FILTER
    /// applies to a stretch's messages.
    fn wants(&self, m: &read::MessageRow, users: &HashMap<String, read::UserRow>) -> bool {
        let f = &self.filter;
        // ASCII case folding, as SQLite's lower() does.
        let from_ok = f.from.as_deref().is_none_or(|raw| {
            let from = raw.to_ascii_lowercase();
            users.get(&m.sender).is_some_and(|u| {
                u.name == raw
                    || u.display_name
                        .as_deref()
                        .is_some_and(|n| n.to_ascii_lowercase().contains(&from))
                    || u.email
                        .as_deref()
                        .is_some_and(|e| e.to_ascii_lowercase().starts_with(&from))
            })
        });
        from_ok && f.before_us.is_none_or(|b| m.create_us < b) && f.after_us.is_none_or(|a| m.create_us >= a)
    }

    /// The words as one phrase, for the embedding model.
    pub fn text(&self) -> String {
        self.terms.join(" ")
    }

    /// FTS5 expression for one term: quoted so user input is literal; prefix-matched when
    /// long enough to be useful ("data" finds "dataset"), and for the last term (still being
    /// typed) from two characters. A one-letter prefix matches nearly everything and makes
    /// FTS walk the whole index.
    fn fts_term(&self, i: usize) -> String {
        let t = &self.terms[i];
        let n = t.chars().count();
        if n >= 3 || (n == 2 && i + 1 == self.terms.len()) {
            format!("\"{t}\"*")
        } else {
            format!("\"{t}\"")
        }
    }

    /// Any term: ranking decides how many must match.
    fn fts_any(&self) -> String {
        self.fts_join(" OR ")
    }

    fn fts_all(&self) -> String {
        self.fts_join(" AND ")
    }

    fn fts_join(&self, op: &str) -> String {
        (0..self.terms.len())
            .map(|i| self.fts_term(i))
            .collect::<Vec<_>>()
            .join(op)
    }
}

pub fn parse(q: &str) -> Query {
    let mut out = Query::default();
    for tok in q.split_whitespace() {
        let (k, v) = tok.split_once(':').unwrap_or(("", tok));
        match k {
            // A bare "from:" (still being typed) filters nothing and is no word either.
            "from" | "in" if v.is_empty() => {}
            "from" => out.filter.from = Some(v.to_owned()),
            "in" => out.filter.space = Some(v.to_owned()),
            "before" => out.filter.before_us = date_us(v),
            "after" => out.filter.after_us = date_us(v),
            _ => {
                let clean: String = tok.chars().filter(|c| *c != '"').collect();
                // A term without letters or digits ("-", "&", an emoji) is no token to FTS:
                // it matches nothing and would keep every stretch short of "all words".
                if clean.chars().any(char::is_alphanumeric) {
                    out.terms.push(clean);
                }
            }
        }
    }
    out
}

fn date_us(s: &str) -> Option<Micros> {
    ts::parse(&format!("{s}T00:00:00Z"))
}

// ------------------------------------------------------------------- index maintenance

/// Rebuilds the stretches of every space whose messages changed. Cheap enough to run before
/// each search, so keyword results are never stale.
pub fn rebuild_dirty(c: &mut Connection) -> Result<usize> {
    let dirty: Vec<String> = {
        let mut st = c.prepare("SELECT name FROM space WHERE index_dirty = 1")?;
        st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    if dirty.is_empty() {
        return Ok(0);
    }
    let users = read::all_users(c)?;
    let tx = c.transaction()?;
    for space in &dirty {
        let msgs: Vec<chunk::Msg> = {
            let mut st = tx.prepare(
                "SELECT id, thread, is_reply, sender, create_us, text FROM message
                 WHERE space = ?1 AND delete_us IS NULL AND local_state = 'synced'
                   AND text IS NOT NULL AND text != ''",
            )?;
            st.query_map([space], |r| {
                let sender: String = r.get(3)?;
                Ok(chunk::Msg {
                    id: r.get(0)?,
                    thread: r.get(1)?,
                    is_reply: r.get(2)?,
                    sender: users
                        .get(&sender)
                        .and_then(|u| u.display_name.clone().or_else(|| u.email.clone()))
                        .unwrap_or_else(|| "Someone".into()),
                    create_us: r.get(4)?,
                    text: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        // Diff against what is stored: a new message changes one stretch, not the space, so
        // only changed stretches are rewritten (and the FTS index with them).
        type Key = (Option<String>, String, String, String);
        let mut stored: HashMap<Key, i64> = {
            let mut st = tx.prepare("SELECT id, thread, msg_ids, text, hash FROM chunk WHERE space = ?1")?;
            st.query_map([space], |r| {
                Ok(((r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?), r.get(0)?))
            })?
            .collect::<rusqlite::Result<_>>()?
        };
        let mut ins = tx.prepare(
            "INSERT INTO chunk (space, thread, first_us, last_us, msg_ids, text, embed, hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for ch in chunk::build(&msgs) {
            let ids = serde_json::to_string(&ch.msg_ids)?;
            // The hash covers the model text, which also holds the thread root's context.
            let key = (ch.thread.clone(), ids.clone(), ch.text.clone(), ch.hash.clone());
            if stored.remove(&key).is_some() {
                continue;
            }
            ins.execute(params![
                space,
                ch.thread,
                ch.first_us,
                ch.last_us,
                ids,
                ch.text,
                ch.embed,
                ch.hash
            ])?;
        }
        let mut del = tx.prepare("DELETE FROM chunk WHERE id = ?1")?;
        for id in stored.into_values() {
            del.execute([id])?;
        }
        tx.execute("UPDATE space SET index_dirty = 0 WHERE name = ?1", [space])?;
    }
    tx.commit()?;
    Ok(dirty.len())
}

/// (hash, model text) of stretches with no vector for `model` yet, newest first (recent talk
/// matters most).
pub fn unembedded(c: &Connection, model: &str, limit: u32) -> Result<Vec<(String, String)>> {
    let mut st = c.prepare(
        "SELECT hash, embed FROM chunk ch
         WHERE NOT EXISTS (SELECT 1 FROM embedding e WHERE e.hash = ch.hash AND e.model = ?1)
         GROUP BY hash ORDER BY max(last_us) DESC LIMIT ?2",
    )?;
    Ok(st
        .query_map(params![model, limit], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?)
}

/// Stores vectors for texts some stretch still has; returns those stored. A batch that was
/// being embedded while the text went (an edit, a logout wipe) is dropped.
pub fn store_vectors(
    c: &mut Connection,
    model: &str,
    vecs: Vec<(String, Vec<f32>)>,
) -> Result<Vec<(String, Vec<f32>)>> {
    let tx = c.transaction()?;
    let mut kept = Vec::with_capacity(vecs.len());
    {
        let mut st = tx.prepare(
            "INSERT OR REPLACE INTO embedding (hash, model, vec)
             SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM chunk WHERE hash = ?1)",
        )?;
        for (h, v) in vecs {
            if st.execute(params![h, model, embed::to_blob(&v)])? > 0 {
                kept.push((h, v));
            }
        }
    }
    tx.commit()?;
    Ok(kept)
}

pub fn load_vectors(c: &Connection, model: &str) -> Result<HashMap<String, Vec<f32>>> {
    let mut st = c.prepare("SELECT hash, vec FROM embedding WHERE model = ?1")?;
    let rows = st.query_map([model], |r| {
        Ok((r.get::<_, String>(0)?, embed::from_blob(&r.get::<_, Vec<u8>>(1)?)))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Drops vectors of text that no stretch has any more (edited, deleted, re-split); returns
/// their hashes.
pub fn drop_orphan_vectors(c: &Connection) -> Result<Vec<String>> {
    let mut st = c.prepare("DELETE FROM embedding WHERE hash NOT IN (SELECT hash FROM chunk) RETURNING hash")?;
    Ok(st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
}

/// (distinct stretch texts, how many of those have a vector for `model`).
pub fn counts(c: &Connection, model: &str) -> Result<(u32, u32)> {
    Ok(c.query_row(
        "SELECT count(DISTINCT hash),
                (SELECT count(*) FROM embedding e WHERE e.model = ?1 AND e.hash IN (SELECT hash FROM chunk))
         FROM chunk",
        [model],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

pub fn history_left(c: &Connection) -> Result<u32> {
    Ok(
        c.query_row("SELECT count(*) FROM space WHERE history_complete = 0", [], |r| {
            r.get(0)
        })?,
    )
}

// ------------------------------------------------------------------- ranking

#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub chunk: i64,
    pub words: bool,
    pub meaning: bool,
}

/// Filters on a `chunk ch` row; parameters ?1..?4 are from, space, before, after. `from` and
/// `in` take a resource name (what the search box's suggestions insert) or typed text, matched
/// literally (no LIKE: "_" in a name is no wildcard) and the same way as `Query::wants`.
const FILTER: &str = "
    LEFT JOIN space s ON s.name = ch.space
    LEFT JOIN user peer ON peer.name = s.dm_peer
    WHERE (?1 IS NULL OR EXISTS (
            SELECT 1 FROM json_each(ch.msg_ids) j JOIN message m ON m.id = j.value
            JOIN user u ON u.name = m.sender
            WHERE u.name = ?1 OR instr(lower(u.display_name), lower(?1)) > 0
               OR substr(lower(u.email), 1, length(?1)) = lower(?1)))
      AND (?2 IS NULL OR s.name = ?2 OR instr(lower(s.display_name), lower(?2)) > 0
           OR instr(lower(peer.display_name), lower(?2)) > 0)
      AND (?3 IS NULL OR ch.first_us < ?3)
      AND (?4 IS NULL OR ch.last_us >= ?4)";

/// Ranks stretches for `q`. `query_vec` and `vectors` enable meaning; without them this is
/// keyword relevance alone.
pub fn rank(
    c: &Connection,
    q: &Query,
    query_vec: Option<&[f32]>,
    vectors: Option<&HashMap<String, Vec<f32>>>,
) -> Result<Vec<Ranked>> {
    let f = &q.filter;
    if q.terms.is_empty() {
        if !q.has_filter() {
            return Ok(vec![]);
        }
        // Filters alone ("from:Noor"): the matching stretches, newest first.
        let sql = format!("SELECT ch.id FROM chunk ch {FILTER} ORDER BY ch.last_us DESC LIMIT 500");
        let mut st = c.prepare(&sql)?;
        return Ok(st
            .query_map(params![f.from, f.space, f.before_us, f.after_us], |r| r.get(0))?
            .map(|id| {
                id.map(|chunk| Ranked {
                    chunk,
                    words: true,
                    meaning: false,
                })
            })
            .collect::<rusqlite::Result<_>>()?);
    }

    // Keyword: candidates matching any term, ordered by how many distinct terms the stretch
    // holds, then bm25 (rarer words and denser matches first), then recency.
    let n = q.terms.len();
    let candidates = |expr: String| -> Result<Vec<(i64, usize, f64, Micros)>> {
        let sql = format!(
            "SELECT ch.id, bm25(chunk_fts), ch.last_us FROM chunk_fts JOIN chunk ch ON ch.id = chunk_fts.rowid
             {FILTER} AND chunk_fts MATCH ?5 ORDER BY bm25(chunk_fts) LIMIT 500"
        );
        let mut st = c.prepare(&sql)?;
        Ok(st
            .query_map(params![f.from, f.space, f.before_us, f.after_us, expr], |r| {
                Ok((r.get::<_, i64>(0)?, 0usize, r.get::<_, f64>(1)?, r.get::<_, Micros>(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?)
    };
    // Stretches with every word first, so two common words cannot push them past the limit;
    // then those with some.
    let mut kw = candidates(q.fts_all())?;
    kw.iter_mut().for_each(|k| k.1 = n);
    if n > 1 {
        let have: HashSet<i64> = kw.iter().map(|k| k.0).collect();
        let mut some: Vec<_> = candidates(q.fts_any())?
            .into_iter()
            .filter(|k| !have.contains(&k.0))
            .collect();
        let mut st = c.prepare("SELECT rowid FROM chunk_fts WHERE chunk_fts MATCH ?1")?;
        let sets: Vec<HashSet<i64>> = (0..n)
            .map(|i| {
                st.query_map([q.fts_term(i)], |r| r.get(0))
                    .and_then(|rows| rows.collect::<rusqlite::Result<HashSet<i64>>>())
            })
            .collect::<rusqlite::Result<_>>()?;
        for k in &mut some {
            k.1 = sets.iter().filter(|s| s.contains(&k.0)).count();
        }
        kw.extend(some);
    }
    kw.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.total_cmp(&b.2)).then(b.3.cmp(&a.3)));
    // The best all-words matches lead. Past a handful, the rest compete with meaning, or a
    // one-word query (where every hit has all words) would never show a related result.
    let full: Vec<bool> = kw.iter().enumerate().map(|(i, k)| k.1 == n && i < FULL_LEAD).collect();

    // Meaning: nearest stretches among those passing the filters, kept only near the best.
    let mut sem: Vec<(i64, f32)> = Vec::new();
    if let (Some(qv), Some(vectors)) = (query_vec, vectors) {
        let sql = format!("SELECT ch.id, ch.hash FROM chunk ch {FILTER}");
        let mut st = c.prepare(&sql)?;
        let rows = st.query_map(params![f.from, f.space, f.before_us, f.after_us], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, hash) = row?;
            if let Some(v) = vectors.get(&hash) {
                sem.push((id, embed::dot(qv, v)));
            }
        }
        sem.sort_by(|a, b| b.1.total_cmp(&a.1));
        sem.truncate(MEANING_TOP);
        if let Some(best) = sem.first().map(|s| s.1) {
            sem.retain(|s| s.1 >= best - MEANING_BAND);
        }
    }

    Ok(fuse(
        &kw.iter().zip(&full).map(|(k, f)| (k.0, *f)).collect::<Vec<_>>(),
        &sem.iter().map(|s| s.0).collect::<Vec<_>>(),
    ))
}

/// Merges the keyword list (`(chunk, has every word)`) and the meaning list. Stretches with
/// every word come first; within that group and below it, reciprocal rank fusion orders by
/// both lists, so meaning reorders and adds but never buries an all-words match.
pub fn fuse(words: &[(i64, bool)], meaning: &[i64]) -> Vec<Ranked> {
    #[derive(Default)]
    struct S {
        score: f32,
        words: bool,
        full: bool,
        meaning: bool,
    }
    let mut score: HashMap<i64, S> = HashMap::new();
    for (i, (id, full)) in words.iter().enumerate() {
        let e = score.entry(*id).or_default();
        e.score += 1.0 / (RRF_K + i as f32 + 1.0);
        e.words = true;
        e.full = *full;
    }
    for (i, id) in meaning.iter().enumerate() {
        let e = score.entry(*id).or_default();
        e.score += 1.0 / (RRF_K + i as f32 + 1.0);
        e.meaning = true;
    }
    let mut out: Vec<(i64, S)> = score.into_iter().collect();
    // Ties go to the keyword list, then by id so the order is stable.
    out.sort_by(|a, b| {
        b.1.full
            .cmp(&a.1.full)
            .then(b.1.score.total_cmp(&a.1.score))
            .then(b.1.words.cmp(&a.1.words))
            .then(b.0.cmp(&a.0))
    });
    out.into_iter()
        .map(|(chunk, s)| Ranked {
            chunk,
            words: s.words,
            meaning: s.meaning,
        })
        .collect()
}

// ------------------------------------------------------------------- results

/// Builds the result rows for ranked stretches.
pub fn hits(c: &Connection, me: &str, q: &Query, ranked: &[Ranked]) -> Result<Vec<SearchHit>> {
    let users = read::all_users(c)?;
    let b = Builder { me, users: &users };
    let names = space_names(c, &b)?;
    let mut found = Vec::with_capacity(ranked.len());
    for r in ranked {
        let row = c
            .query_row(
                "SELECT space, thread, first_us, last_us, msg_ids FROM chunk WHERE id = ?1",
                [r.chunk],
                |x| {
                    Ok((
                        x.get::<_, String>(0)?,
                        x.get::<_, Option<String>>(1)?,
                        x.get::<_, Micros>(2)?,
                        x.get::<_, Micros>(3)?,
                        x.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        if let Some((space, thread, first_us, last_us, ids)) = row {
            found.push((
                r,
                space,
                thread,
                first_us,
                last_us,
                serde_json::from_str::<Vec<i64>>(&ids)?,
            ));
        }
    }
    // Highlights for every message of the word-matched stretches in one query: FTS5 does not
    // narrow its scan by rowid, so a query per stretch would scan the index per stretch.
    let mut snips: HashMap<i64, String> = HashMap::new();
    let word_ids: Vec<i64> = found
        .iter()
        .filter(|f| f.0.words)
        .flat_map(|f| f.5.iter().copied())
        .collect();
    if !word_ids.is_empty() && !q.terms.is_empty() {
        let mut st = c.prepare(
            "SELECT rowid, snippet(message_fts, 0, char(1), char(2), '…', 14) FROM message_fts
             WHERE message_fts MATCH ?1 AND rowid IN (SELECT value FROM json_each(?2))",
        )?;
        let rows = st.query_map(params![q.fts_any(), serde_json::to_string(&word_ids)?], |x| {
            Ok((x.get::<_, i64>(0)?, x.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, s) = row?;
            snips.insert(id, s);
        }
    }
    let mut out = Vec::with_capacity(found.len());
    for (r, space, thread, first_us, last_us, ids) in found {
        let rows = read::messages_by_id(c, &ids)?;
        if rows.is_empty() {
            continue;
        }
        // Only messages from the asked sender and dates are shown; the stretch as a whole
        // matched, but a line from someone else under "from:Noor" would read as wrong.
        let mut pool: Vec<&read::MessageRow> = rows.iter().filter(|m| q.wants(m, &users)).collect();
        if pool.is_empty() {
            pool = rows.iter().collect();
        }
        let has_snip = r.words && pool.iter().any(|m| snips.contains_key(&m.id));
        // Lines with a word match; else (matched by meaning, or by filters alone) the first.
        let shown: Vec<&read::MessageRow> = if has_snip {
            pool.into_iter()
                .filter(|m| snips.contains_key(&m.id))
                .take(LINES)
                .collect()
        } else {
            pool.into_iter().take(LINES).collect()
        };
        let lines: Vec<SearchLine> = shown
            .iter()
            .map(|m| SearchLine {
                message: m.name.clone(),
                sender: b.user(&m.sender),
                create_us: m.create_us,
                parts: match snips.get(&m.id) {
                    Some(s) => snippet_parts(s),
                    None => vec![SnippetPart {
                        text: chunk::clip(m.text.as_deref().unwrap_or("").trim(), PLAIN_LINE),
                        hit: false,
                    }],
                },
            })
            .collect();
        out.push(SearchHit {
            space_name: names.get(&space).cloned().unwrap_or_default(),
            space,
            thread,
            anchor: shown.first().map(|m| m.name.clone()).unwrap_or_default(),
            first_us,
            last_us,
            more: (rows.len() - lines.len()) as u32,
            lines,
            words: r.words,
            meaning: r.meaning,
        });
    }
    Ok(out)
}

fn space_names(c: &Connection, b: &Builder) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for s in read::spaces(c, b.me)? {
        let members = if s.display_name.is_none() && s.space_type != "DIRECT_MESSAGE" {
            read::member_names(c, &s.name).unwrap_or_default()
        } else {
            vec![]
        };
        out.insert(s.name.clone(), b.space(&s, &members, None).display_name);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser() {
        let q = parse("from:ada in:tech before:2024-06-01 deploy \"green\" - 👍 x");
        assert_eq!(q.terms, vec!["deploy", "green", "x"], "symbol-only terms are dropped");
        assert_eq!(q.fts_any(), "\"deploy\"* OR \"green\"* OR \"x\"");
        assert_eq!(q.fts_all(), "\"deploy\"* AND \"green\"* AND \"x\"");
        assert_eq!(q.filter.from.as_deref(), Some("ada"));
        assert_eq!(q.filter.space.as_deref(), Some("tech"));
        assert!(q.filter.before_us.is_some());
        let q = parse("data from: in:");
        assert_eq!(q.terms, vec!["data"], "a bare from: is neither a filter nor a word");
        assert!(!q.has_filter());
        // Two-letter words match whole, except while being typed (last); one letter never
        // prefix-matches (it would walk the whole index).
        let q = parse("ab cd");
        assert_eq!(q.fts_any(), "\"ab\" OR \"cd\"*");
    }

    #[test]
    fn fusion_rewards_agreement() {
        let r = fuse(&[(1, true), (2, true), (3, true)], &[3, 4]);
        assert_eq!(r[0].chunk, 3, "in both lists beats first of one");
        assert!(r[0].words && r[0].meaning);
        assert_eq!(r[1].chunk, 1);
        assert!(r.iter().any(|x| x.chunk == 4 && !x.words && x.meaning));
    }

    #[test]
    fn all_words_matches_stay_on_top() {
        // 5 has only some of the words but is the best meaning match; 6 has them all.
        let r = fuse(&[(6, true), (5, false)], &[5, 7]);
        assert_eq!(r.iter().map(|x| x.chunk).collect::<Vec<_>>(), vec![6, 5, 7]);
    }
}
