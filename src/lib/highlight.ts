// Syntax colours for code blocks. Chat code blocks carry no language, so it is guessed from
// a short list of common ones, and only a confident guess is coloured. Loaded on first use.
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import go from "highlight.js/lib/languages/go";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

const LANGUAGES = { bash, css, diff, go, java, javascript, json, python, rust, sql, typescript, xml, yaml };
for (const [name, lang] of Object.entries(LANGUAGES)) hljs.registerLanguage(name, lang);

/** highlight.js's relevance score. Sampled: prose scores 0 to 2; short Rust 5, TypeScript 7;
 * a short Python snippet scores 4 as CSS. Below 5 is too often a wrong guess. */
const CONFIDENT = 5;

/** The code as HTML with colour spans, or null when the language isn't clear. highlight.js
 * escapes the text itself, so the result is safe to put in the page. */
export function highlight(code: string): string | null {
  if (code.length > 20_000) return null;
  const r = hljs.highlightAuto(code, Object.keys(LANGUAGES));
  return r.language && r.relevance >= CONFIDENT ? r.value : null;
}
