//! Phase 0 API spike and manual smoke test. Not shipped.
//!
//! Always uses a file token store under the app data dir, so rebuilding never triggers a
//! Keychain prompt. Raw responses are written to `secrets/fixtures/` (gitignored: they hold
//! real message content) for building scrubbed replay fixtures later.
//!
//!   cargo run -p kwek-core --bin spike -- <command>
//!
//! Commands: login | me | spaces | messages <space> [n] | members <space> | readstate <space>
//!   | threadstate <thread> | events <space> <rfc3339> | reactions <message> | subscribe
//!   | pull [seconds] | send <space> <text> | get <url> | logout

use std::sync::Arc;

use kwek_core::auth::{FileStore, TokenManager};
use kwek_core::chat::{ChatApi, HttpChat, ListMessages};
use kwek_core::events::{EventsClient, SPACES_EVENT_TYPES, SPACES_TARGET, USER_EVENT_TYPES, user_target};
use kwek_core::google::Google;
use kwek_core::people::{HttpPeople, PeopleApi};
use kwek_core::render::{self, Ctx};
use kwek_core::{Error, config, default_data_dir, http_client};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../secrets/fixtures");

fn save(name: &str, v: &serde_json::Value) {
    std::fs::create_dir_all(FIXTURES).ok();
    let path = format!("{FIXTURES}/{}.json", name.replace('/', "_"));
    std::fs::write(&path, serde_json::to_string_pretty(v).unwrap()).ok();
    eprintln!("(saved {path})");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let arg = |i: usize| args.get(i).cloned().ok_or_else(|| anyhow::anyhow!("missing argument {i}"));

    let http = http_client();
    let data = default_data_dir();
    let store = Box::new(FileStore::new(data.join("dev-refresh-token")));
    let tokens = Arc::new(TokenManager::new(http.clone(), config::oauth_client().ok(), store));
    let g = Google::new(http.clone(), tokens.clone());
    let chat = HttpChat::new(g.clone());
    let people = HttpPeople::new(g.clone());
    let events = EventsClient::new(g.clone());

    match cmd {
        "login" => {
            tokens
                .login(|url| {
                    eprintln!("Opening the browser for sign-in. If it does not open:\n{url}\n");
                    std::process::Command::new("open").arg(url).status().map(|_| ()).map_err(Error::from)
                })
                .await?;
            let me = people.me().await?;
            println!("signed in as {} ({})", me.display_name.unwrap_or_default(), me.user);
        }
        "logout" => tokens.logout().await?,
        "me" => {
            let v: serde_json::Value = g
                .get("https://people.googleapis.com/v1/people/me?personFields=names,emailAddresses,photos,metadata")
                .await?;
            save("people_me", &v);
            println!("{:#?}", people.me().await?);
        }
        "spaces" => {
            let mut token = None;
            let mut n = 0;
            loop {
                let page = chat.list_spaces(token.as_deref()).await?;
                for s in &page.items {
                    n += 1;
                    println!(
                        "{:<22} {:<15} {:<20} {:<28} {}",
                        s.name,
                        s.kind(),
                        s.space_threading_state.as_deref().unwrap_or("-"),
                        s.last_active_time.as_deref().unwrap_or("-"),
                        s.display_name.as_deref().unwrap_or("")
                    );
                }
                token = page.next().map(str::to_owned);
                if token.is_none() {
                    break;
                }
            }
            let raw: serde_json::Value = g.get("https://chat.googleapis.com/v1/spaces?pageSize=50").await?;
            save("spaces_list", &raw);
            println!("{n} spaces");
        }
        "messages" => {
            let space = arg(1)?;
            let n: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
            let raw: serde_json::Value = g
                .get(&format!(
                    "https://chat.googleapis.com/v1/{space}/messages?pageSize={n}&orderBy=createTime%20desc&showDeleted=true"
                ))
                .await?;
            save(&format!("messages_{space}"), &raw);
            let page = chat
                .list_messages(&space, &ListMessages { page_size: n, newest_first: true, ..Default::default() })
                .await?;
            let mut senders: Vec<String> =
                page.items.iter().filter_map(|m| m.sender.as_ref().map(|s| s.name.clone())).collect();
            senders.sort();
            senders.dedup();
            let with_names = page
                .items
                .iter()
                .filter(|m| m.sender.as_ref().and_then(|s| s.display_name.as_ref()).is_some_and(|d| !d.is_empty()))
                .count();
            let resolved = people.resolve(&senders).await?;
            println!(
                "{} messages, {} distinct senders, {} messages carry sender.displayName, People resolved {}/{}",
                page.items.len(),
                senders.len(),
                with_names,
                resolved.len(),
                senders.len()
            );
            let me = people.me().await?;
            let lookup = |u: &str| resolved.iter().find(|p| p.user == u).and_then(|p| p.display_name.clone());
            for m in page.items.iter().rev() {
                let who = m.sender.as_ref().map(|s| s.name.as_str()).unwrap_or("?");
                let src = m.formatted_text.as_deref().or(m.text.as_deref()).unwrap_or("");
                let segs = render::parse(src, &Ctx { me: &me.user, name_of: &lookup });
                let lookup = |u: &str| lookup(u).or_else(|| m.sender.as_ref().and_then(|s| s.display_name.clone()));
                println!(
                    "[{}] {}{} {}: {}",
                    m.create_time.as_deref().unwrap_or("?"),
                    if m.thread_reply == Some(true) { "  ↳ " } else { "" },
                    if m.delete_time.is_some() { "(deleted)" } else { "" },
                    lookup(who).unwrap_or_else(|| who.to_owned()),
                    render::plain(&segs).replace('\n', " ⏎ ")
                );
            }
            for p in &resolved {
                println!("  {} -> {:?} {:?}", p.user, p.display_name, p.email);
            }
        }
        "members" => {
            let space = arg(1)?;
            let raw: serde_json::Value =
                g.get(&format!("https://chat.googleapis.com/v1/{space}/members?pageSize=100")).await?;
            save(&format!("members_{space}"), &raw);
            println!("{}", serde_json::to_string_pretty(&raw)?);
        }
        "readstate" => {
            let space = arg(1)?;
            println!("{:#?}", chat.get_space_read_state(&space).await?);
        }
        "threadstate" => {
            let thread = arg(1)?;
            println!("{:#?}", chat.get_thread_read_state(&thread).await?);
        }
        "events" => {
            let (space, since) = (arg(1)?, arg(2)?);
            let page = chat.list_space_events(&space, &since, None).await?;
            for e in &page.items {
                let decoded = kwek_core::events::decode_space_event(e);
                println!("{} {} -> {} decoded", e.event_time.as_deref().unwrap_or("?"), e.event_type, decoded.len());
            }
            println!("{} events, more: {}", page.items.len(), page.next().is_some());
        }
        "reactions" => {
            let msg = arg(1)?;
            let me = people.me().await?;
            let raw: serde_json::Value =
                g.get(&format!("https://chat.googleapis.com/v1/{msg}/reactions?pageSize=50")).await?;
            save("reactions", &raw);
            println!("all: {}", serde_json::to_string_pretty(&raw)?);
            println!("mine: {:#?}", chat.list_reactions_by(&msg, &me.user).await?);
        }
        "subscribe" => {
            let me = people.me().await?;
            let device = "spike";
            let sub = events.ensure_pubsub(device).await?;
            println!("pubsub: {sub}");
            let s = events.ensure_subscription(SPACES_TARGET, SPACES_EVENT_TYPES, true).await?;
            println!("spaces subscription: {} state={:?} expires={:?}", s.name, s.state, s.expire_time);
            match events.ensure_subscription(&user_target(&me.user), USER_EVENT_TYPES, true).await {
                Ok(s) => println!("user subscription: {} state={:?} expires={:?}", s.name, s.state, s.expire_time),
                Err(e) => {
                    println!("user subscription with includeResource=true failed: {e}\nretrying without resource");
                    let s = events.ensure_subscription(&user_target(&me.user), USER_EVENT_TYPES, false).await?;
                    println!("user subscription: {} state={:?} expires={:?}", s.name, s.state, s.expire_time);
                }
            }
        }
        "pull" => {
            let secs: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
            let sub = kwek_core::events::device_subscription("spike");
            let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
            let mut i = 0;
            while std::time::Instant::now() < until {
                let batch = events.pull(&sub, 100).await?;
                let mut acks = Vec::new();
                for r in &batch {
                    i += 1;
                    let raw = serde_json::json!({
                        "attributes": r.message.attributes,
                        "publishTime": r.message.publish_time,
                        "data": r.message.data.as_deref()
                            .and_then(|d| base64::Engine::decode(&base64::engine::general_purpose::STANDARD, d).ok())
                            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()),
                    });
                    save(&format!("event_{i}_{}", r.ce_type()), &raw);
                    println!(
                        "{} {} published {:?} -> {:?}",
                        r.ce_type(),
                        r.message.attributes.get("ce-subject").map(String::as_str).unwrap_or(""),
                        r.message.publish_time,
                        r.decode().iter().map(|e| format!("{e:?}").chars().take(100).collect::<String>()).collect::<Vec<_>>()
                    );
                    acks.push(r.ack_id.clone());
                }
                events.ack(&sub, &acks).await?;
            }
        }
        "send" => {
            let (space, text) = (arg(1)?, arg(2)?);
            let s = chat.get_space(&space).await?;
            if s.display_name.as_deref() != Some("kwek-test") {
                anyhow::bail!("refusing to send outside the kwek-test space (this is {:?})", s.display_name);
            }
            let id = format!("client-{}", uuid::Uuid::new_v4());
            let m = chat.create_message(&space, &text, None, &id).await?;
            println!("{:#?}", m);
        }
        "get" => {
            let v: serde_json::Value = g.get(&arg(1)?).await?;
            println!("{}", serde_json::to_string_pretty(&v)?);
        }
        _ => eprintln!("see the module docs at the top of crates/kwek-core/src/bin/spike.rs"),
    }
    Ok(())
}
