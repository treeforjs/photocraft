//! File › Automate › Batch (`file.automate.batch`): run a recorded action over a folder (and its
//! subfolders), a list of files or the open documents, saving each result under a naming
//! template.
//!
//! A background job when started with [`Session::start`]: the files are processed on a worker,
//! with progress per file and step, and Cancel stops before the next step. Files already
//! written stay written (as in Photoshop); the open documents are never changed, each one is
//! processed as a copy.

use std::sync::Arc;

use photocraft_doc::Document;
use serde_json::{Value, json};

use crate::file_cmds::{OutputClaims, SaveOpts, file_name, import, join, read_file, save_doc, stem, str_param};
use crate::jobs::JobCtx;
use crate::{EngineError, Result, Session};

const CMD: &str = "file.automate.batch";

/// Deepest subfolder level "Include All Subfolders" descends to (a guard against cycles and
/// pathological trees; symlinked folders are never followed).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const MAX_DEPTH: usize = 32;

/// Most files one batch takes (a guard against a mistyped root folder).
const MAX_INPUTS: usize = 100_000;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: msg.into() }
}

/// Where one input comes from.
enum Source {
    File(String),
    /// An open document, processed as a copy.
    Open {
        name: String,
        path: Option<String>,
        doc: Arc<Document>,
    },
}

struct Input {
    source: Source,
    /// The input's subfolder below the source folder (`/`-separated, empty at the top). The
    /// result goes to the same subfolder of the destination.
    rel: String,
}

impl Input {
    fn label(&self) -> String {
        match &self.source {
            Source::File(p) => p.clone(),
            Source::Open { name, .. } => name.clone(),
        }
    }

    fn name(&self) -> String {
        match &self.source {
            Source::File(p) => file_name(p),
            Source::Open { name, .. } => name.clone(),
        }
    }
}

/// Images in `dir`, sorted by name, then (with `recurse`) those of each subfolder in name
/// order.
#[cfg(not(target_arch = "wasm32"))]
fn list_tree(dir: &str, recurse: bool, rel: &str, depth: usize, out: &mut Vec<Input>) -> Result<()> {
    for path in crate::file_cmds::list_images(dir)? {
        if out.len() >= MAX_INPUTS {
            return Err(bad(format!("more than {MAX_INPUTS} input files")));
        }
        out.push(Input { source: Source::File(path), rel: rel.to_string() });
    }
    if !recurse || depth >= MAX_DEPTH {
        return Ok(());
    }
    let rd = std::fs::read_dir(dir).map_err(|e| EngineError::Other(format!("{dir}: {e}")))?;
    let mut subs: Vec<(String, String)> = rd
        .filter_map(|e| e.ok())
        // `file_type` does not follow symlinks: a linked folder is skipped, so a link cycle
        // can't loop.
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            (!name.starts_with('.')).then(|| (name, e.path().to_string_lossy().into_owned()))
        })
        .collect();
    subs.sort();
    for (name, path) in subs {
        let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
        list_tree(&path, true, &rel, depth + 1, out)?;
    }
    Ok(())
}
#[cfg(target_arch = "wasm32")]
fn list_tree(dir: &str, _recurse: bool, _rel: &str, _depth: usize, _out: &mut Vec<Input>) -> Result<()> {
    Err(EngineError::Other(format!("cannot list {dir}: no file system on the web")))
}

fn inputs(s: &Session, p: &Value) -> Result<Vec<Input>> {
    let recurse = p.get("includeSubfolders").and_then(Value::as_bool).unwrap_or(false);
    let mut out = Vec::new();
    match p.get("input").or_else(|| p.get("files")) {
        Some(Value::String(v)) if v == "open" => {
            for d in s.documents() {
                out.push(Input { source: Source::Open { name: d.doc.name.clone(), path: d.path.clone(), doc: d.doc.clone() }, rel: String::new() });
            }
            if out.is_empty() {
                return Err(bad("no open documents"));
            }
        }
        Some(Value::String(dir)) => list_tree(dir, recurse, "", 0, &mut out)?,
        Some(Value::Array(a)) => {
            out.extend(a.iter().filter_map(Value::as_str).take(MAX_INPUTS).map(|f| Input { source: Source::File(f.to_string()), rel: String::new() }))
        }
        _ => return Err(bad("missing \"input\" (folder, array of files, or \"open\" for the open documents)")),
    }
    Ok(out)
}

/// A token of a naming template.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Text(String),
    /// The input's name without extension, as is, in capitals, or in lower case.
    Name,
    NameUpper,
    NameLower,
    /// The serial number, zero-padded to the width.
    Serial(usize),
    /// The serial as letters: a…z, aa, ab… (`upper` for A…Z).
    Letter {
        upper: bool,
    },
    /// The run's date in a fixed layout (`yyyymmdd`, `yymmdd`, `mmddyy`, `mmdd`, `ddmmyy`,
    /// `ddmm`).
    Date(&'static str),
    /// The output extension, as is or in capitals.
    Ext,
    ExtUpper,
}

const DATE_LAYOUTS: [&str; 6] = ["yyyymmdd", "yymmdd", "mmddyy", "mmdd", "ddmmyy", "ddmm"];

/// Parses a template such as `{name}_{serial:3}`. Text outside braces is kept; `{{` and `}}`
/// are literal braces.
fn parse_template(t: &str) -> Result<Vec<Token>> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut tok = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => tok.push(c),
                        None => return Err(bad(format!("naming: unclosed `{{{tok}`"))),
                    }
                }
                if !text.is_empty() {
                    out.push(Token::Text(std::mem::take(&mut text)));
                }
                out.push(token(&tok)?);
            }
            '}' => return Err(bad("naming: unmatched `}` (write `}}` for a brace)")),
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        out.push(Token::Text(text));
    }
    if out.is_empty() {
        return Err(bad("naming: the template is empty"));
    }
    Ok(out)
}

fn token(t: &str) -> Result<Token> {
    Ok(match t {
        "name" => Token::Name,
        "NAME" => Token::NameUpper,
        "lname" => Token::NameLower,
        "serial" => Token::Serial(1),
        "letter" => Token::Letter { upper: false },
        "LETTER" => Token::Letter { upper: true },
        "ext" => Token::Ext,
        "EXT" => Token::ExtUpper,
        _ => {
            if let Some(layout) = DATE_LAYOUTS.into_iter().find(|l| *l == t) {
                Token::Date(layout)
            } else if let Some(w) = t.strip_prefix("serial:") {
                let w: usize = w.parse().ok().filter(|w| (1..=9).contains(w)).ok_or_else(|| bad(format!("naming: `{{{t}}}` needs a width of 1 to 9")))?;
                Token::Serial(w)
            } else {
                return Err(bad(format!(
                    "naming: unknown token `{{{t}}}` (name, NAME, lname, serial, serial:N, letter, LETTER, ext, EXT, {})",
                    DATE_LAYOUTS.join(", ")
                )));
            }
        }
    })
}

/// a…z, aa…zz, aaa… for 1, 2, 3… (0 gives an empty string).
fn letters(mut n: u64, upper: bool) -> String {
    let base = if upper { b'A' } else { b'a' };
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(char::from(base + (n % 26) as u8));
        n /= 26;
    }
    out.iter().rev().collect()
}

struct Naming {
    tokens: Vec<Token>,
    start: u64,
    date: (i64, u32, u32),
}

impl Naming {
    fn from_params(p: &Value) -> Result<Self> {
        let tokens = parse_template(p.get("naming").and_then(Value::as_str).unwrap_or("{name}"))?;
        let start = match p.get("startSerial") {
            None => 1,
            Some(v) => v.as_u64().filter(|n| *n <= 1_000_000_000).ok_or_else(|| bad("\"startSerial\" must be a whole number from 0"))?,
        };
        let date = match p.get("date").and_then(Value::as_str) {
            Some(d) => parse_date(d).ok_or_else(|| bad("\"date\" must be YYYY-MM-DD"))?,
            None => parse_date(&crate::analysis_cmds::now_iso()).unwrap_or((1970, 1, 1)),
        };
        Ok(Naming { tokens, start, date })
    }

    /// The output file name (without folder) of the `index`th input.
    fn render(&self, name: &str, ext: &str, index: usize) -> Result<String> {
        let serial = self.start.saturating_add(index as u64);
        let (y, m, d) = self.date;
        let yy = y.rem_euclid(100);
        let mut out = String::new();
        for t in &self.tokens {
            match t {
                Token::Text(s) => out.push_str(s),
                Token::Name => out.push_str(name),
                Token::NameUpper => out.push_str(&name.to_uppercase()),
                Token::NameLower => out.push_str(&name.to_lowercase()),
                Token::Serial(w) => out.push_str(&format!("{serial:0w$}")),
                Token::Letter { upper } => out.push_str(&letters(serial, *upper)),
                Token::Date(layout) => out.push_str(&match *layout {
                    "yyyymmdd" => format!("{y:04}{m:02}{d:02}"),
                    "yymmdd" => format!("{yy:02}{m:02}{d:02}"),
                    "mmddyy" => format!("{m:02}{d:02}{yy:02}"),
                    "mmdd" => format!("{m:02}{d:02}"),
                    "ddmmyy" => format!("{d:02}{m:02}{yy:02}"),
                    _ => format!("{d:02}{m:02}"),
                }),
                Token::Ext => out.push_str(ext),
                Token::ExtUpper => out.push_str(&ext.to_uppercase()),
            }
        }
        // A name never leaves the destination folder.
        let out: String = out.chars().map(|c| if matches!(c, '/' | '\\' | ':') || c.is_control() { '_' } else { c }).collect();
        let out = out.trim().to_string();
        if out.is_empty() || out.chars().all(|c| c == '.') {
            return Err(EngineError::Other(format!("the naming template gives an empty name for {name}")));
        }
        Ok(format!("{out}.{ext}"))
    }
}

/// `YYYY-MM-DD` (anything after the day is ignored, so an ISO time works too).
fn parse_date(s: &str) -> Option<(i64, u32, u32)> {
    let y = s.get(0..4)?.parse().ok()?;
    let m = s.get(5..7)?.parse().ok().filter(|m| (1..=12).contains(m))?;
    let d = s.get(8..10)?.parse().ok().filter(|d| (1..=31).contains(d))?;
    (s.get(4..5) == Some("-") && s.get(7..8) == Some("-")).then_some((y, m, d))
}

struct Plan {
    steps: Vec<(String, Value)>,
    inputs: Vec<Input>,
    output: String,
    format: String,
    save: SaveOpts,
    naming: Naming,
    stop_on_error: bool,
    error_log: Option<String>,
}

pub(crate) fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    let steps = crate::file_cmds::parse_steps(p.get("steps").or_else(|| p.get("action")).ok_or_else(|| bad("missing \"steps\""))?)?;
    if let Some((id, _)) = steps.iter().find(|(id, _)| crate::commands::find(id).is_none()) {
        return Err(bad(format!("unknown command `{id}` in the action")));
    }
    let plan = Plan {
        steps,
        inputs: inputs(s, p)?,
        output: str_param(p, "output", CMD)?.to_string(),
        format: p.get("format").and_then(Value::as_str).unwrap_or("same").to_string(),
        save: SaveOpts::from_params(p),
        naming: Naming::from_params(p)?,
        stop_on_error: p.get("stopOnError").and_then(Value::as_bool).unwrap_or(false),
        error_log: p.get("errorLog").and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string),
    };
    crate::jobs::run(s, "Batch", false, move |ctx| run_plan(&plan, ctx), |_, v| Ok(v))
}

fn run_plan(plan: &Plan, ctx: &JobCtx) -> Result<Value> {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut written = OutputClaims::default();
    let n = plan.inputs.len().max(1) as f32;
    let per_file = plan.steps.len() as f32 + 2.0;
    let mut stopped = false;
    for (i, input) in plan.inputs.iter().enumerate() {
        ctx.check()?;
        let label = input.label();
        let at = |k: f32| (i as f32 + k / per_file) / n;
        ctx.progress(at(0.0), &format!("{} ({} of {})", input.name(), i + 1, plan.inputs.len()));
        let r = (|| -> Result<String> {
            let name = input.name();
            let ext = if plan.format == "same" {
                name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_else(|| "psd".into())
            } else {
                plan.format.trim_start_matches('.').to_ascii_lowercase()
            };
            let dir = if input.rel.is_empty() { plan.output.clone() } else { join(&plan.output, &input.rel) };
            let out = join(&dir, &plan.naming.render(&stem(&name), &ext, i)?);
            written.check(&out).map_err(EngineError::Other)?;
            let mut scratch = Session::new();
            match &input.source {
                Source::File(path) => {
                    let bytes = read_file(path)?;
                    scratch.add_document(import(&file_name(path), &bytes)?, Some(path.clone()));
                }
                Source::Open { path, doc, .. } => {
                    scratch.add_document((**doc).clone(), path.clone());
                }
            }
            for (k, (id, params)) in plan.steps.iter().enumerate() {
                ctx.check()?;
                ctx.progress(at(1.0 + k as f32), "");
                scratch.execute(id, params.clone()).map_err(|e| EngineError::Other(format!("step {} ({id}): {e}", k + 1)))?;
            }
            ctx.check()?;
            ctx.progress(at(per_file - 1.0), "");
            let d = scratch.active().ok_or(EngineError::NoDocument)?;
            #[cfg(not(target_arch = "wasm32"))]
            if !input.rel.is_empty() {
                std::fs::create_dir_all(&dir).map_err(|e| EngineError::Other(format!("{dir}: {e}")))?;
            }
            save_doc(&d.doc, &out, plan.save)?;
            Ok(out)
        })();
        match r {
            Ok(out) => {
                written.record(&out, &label);
                files.push(out);
            }
            Err(EngineError::Cancelled) => return Err(EngineError::Cancelled),
            Err(e) => {
                errors.push(json!({"file": label, "error": e.to_string()}));
                if plan.stop_on_error {
                    stopped = true;
                    break;
                }
            }
        }
    }
    ctx.progress(1.0, "");
    let mut r = json!({"files": files, "errors": errors});
    if stopped {
        r["stopped"] = json!(true);
    }
    if let Some(log) = &plan.error_log
        && !errors.is_empty()
    {
        let text: String = errors.iter().map(|e| format!("{}: {}\n", e["file"].as_str().unwrap_or(""), e["error"].as_str().unwrap_or(""))).collect();
        crate::file_cmds::write_file(log, text.as_bytes())?;
        r["errorLog"] = json!(log);
    }
    Ok(r)
}

#[cfg(test)]
mod tests;
