//! Window › Actions, as engine commands (`actions.record` / `stop` / `play` / `list` / `get` /
//! `delete`).
//!
//! The list lives on [`Session`] so the panel, the CLI, the control channel and MCP share one
//! copy. Recording copies replayable journal entries (commands whose [`CommandSpec::journal`] is
//! set, except action-editing commands and the already-expanded `edit.transform.again`) into an
//! action. Playback runs those steps with [`Session::execute`]
//! and stops at the first error, leaving one history step per step that ran.
//!
//! Recorded steps are document-independent, as Photoshop's are: an explicit layer id becomes
//! `{"$layer":"active"}` when it named the layer that was active when the step ran, or
//! `{"$layer":{"name":…}}` otherwise, and [`Session::execute`] resolves those references
//! against whatever document the step replays on. Keystrokes typed into one text layer
//! (consecutive `type.edit` replacements inside the text the previous one inserted) merge into
//! a single step, and history-coalescing keys are dropped.
//!
//! An untrusted session installs [`Session::authorize`]. `actions.play` calls it for every nested
//! step, because the outer `actions.play` id would otherwise hide a recorded `file.*` command from
//! the top-level check.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// One recorded action. `steps` is the `[[id, params], …]` shape `file.automate.batch` and
/// droplets already accept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    #[serde(default)]
    pub steps: Vec<(String, Value)>,
}

/// On-disk form of [`ActionState::list`] (`actions.json` in the preset store).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionsFile {
    pub version: u32,
    pub actions: Vec<Action>,
}

/// Actions list and the recording / playback flags. Only `list` is persisted.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ActionState {
    pub list: Vec<Action>,
    /// Journal length when recording started, and the action index being recorded into.
    #[serde(skip)]
    pub recording: Option<(usize, usize)>,
    /// Nesting depth of `actions.play`, bounded by the playback stack limit.
    #[serde(skip)]
    pub playing: u8,
    /// Active action names, for cycle detection in engine and desktop playback.
    #[serde(skip)]
    pub playback_stack: Vec<String>,
    /// Bumped whenever `list` changes, so the preset store can skip unchanged writes.
    #[serde(skip)]
    pub rev: u64,
    /// Document-independent params for journal entries made while recording, by journal index.
    #[serde(skip)]
    pub(crate) portable: Vec<(usize, Value)>,
}

impl ActionState {
    pub(crate) fn touch(&mut self) {
        self.rev = self.rev.saturating_add(1);
    }
}

/// Replay commands and named action calls, excluding edits to the action definitions and the
/// already-expanded Again wrapper.
pub fn replayable(id: &str) -> bool {
    // Again journals its concrete transform too; recording the wrapper would replay it twice.
    id == "actions.play"
        || (!id.starts_with("actions.") && id != "edit.transform.again" && (shell_view_command(id) || crate::commands::find(id).is_some_and(|c| c.journal)))
}

/// View-only menu commands recorded by the desktop shell. Headless playback reports
/// them as unsupported instead of silently dropping them.
pub fn shell_view_command(id: &str) -> bool {
    matches!(id, "view.fitOnScreen" | "view.actualPixels" | "view.zoomIn" | "view.zoomOut")
}

/// Shared validation for headless and shell playback. Does not start playback.
pub fn playback_plan(s: &Session, p: &Value) -> Result<(Action, usize)> {
    if s.actions.playing > 0 && s.actions.playback_stack.is_empty() {
        return Err(bad("actions.play", "an action is already playing"));
    }
    let idx = resolve(&s.actions.list, p, "actions.play")?;
    let action = s.actions.list.get(idx).cloned().ok_or_else(|| bad("actions.play", "no such action"))?;
    if s.actions.playback_stack.iter().any(|name| name == &action.name) || s.actions.recording.is_some_and(|(_, i)| i == idx) {
        return Err(bad("actions.play", format!("recursive action call to `{}`", action.name)));
    }
    if s.actions.playback_stack.len() >= 16 {
        return Err(bad("actions.play", "action nesting exceeds 16 levels"));
    }
    let from = from_step(p, action.steps.len())?;
    Ok((action, from))
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn num(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().filter(|f| f.is_finite()).map(|f| f.round() as i64))
}

fn resolve(list: &[Action], p: &Value, cmd: &str) -> Result<usize> {
    let Some(v) = p.get("action") else {
        return Err(bad(cmd, "needs \"action\" (a name or an index)"));
    };
    if let Some(i) = num(v) {
        if i < 0 {
            return Err(bad(cmd, "action index is negative"));
        }
        let Some(i) = usize::try_from(i).ok() else {
            return Err(bad(cmd, "action index is out of range"));
        };
        if i >= list.len() {
            return Err(bad(cmd, format!("no action at index {i}")));
        }
        return Ok(i);
    }
    match v.as_str() {
        Some(name) if !name.is_empty() => list.iter().position(|a| a.name == name).ok_or_else(|| bad(cmd, format!("no action named `{name}`"))),
        Some(_) => Err(bad(cmd, "action name is empty")),
        None => Err(bad(cmd, "\"action\" must be a name or an index")),
    }
}

fn from_step(p: &Value, len: usize) -> Result<usize> {
    let Some(v) = p.get("from") else { return Ok(0) };
    if v.is_null() {
        return Ok(0);
    }
    let Some(i) = num(v) else {
        return Err(bad("actions.play", "\"from\" must be a step index"));
    };
    if i < 0 {
        return Err(bad("actions.play", "\"from\" is negative"));
    }
    let Some(i) = usize::try_from(i).ok() else {
        return Err(bad("actions.play", "\"from\" is out of range"));
    };
    if i > len {
        return Err(bad("actions.play", format!("\"from\" {i} is past the last step ({len})")));
    }
    Ok(i)
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let actions: Vec<Value> = s.actions.list.iter().map(|a| json!({"name": a.name, "steps": a.steps.len()})).collect();
    let recording = s.actions.recording.and_then(|(_, i)| s.actions.list.get(i).map(|a| a.name.clone()));
    Ok(json!({"actions": actions, "recording": recording}))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let idx = resolve(&s.actions.list, p, "actions.get")?;
    let action = &s.actions.list[idx];
    let steps: Vec<Value> = action.steps.iter().map(|(id, params)| json!([id, params])).collect();
    Ok(json!({"name": action.name, "steps": steps}))
}

fn record(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.record")?;
    if s.actions.recording.is_some() {
        return Err(bad("actions.record", "already recording; stop first"));
    }
    let idx = if p.get("action").is_some() {
        resolve(&s.actions.list, p, "actions.record")?
    } else {
        let name = match p.get("name") {
            None | Some(Value::Null) => {
                let mut n = s.actions.list.len().saturating_add(1);
                while s.actions.list.iter().any(|a| a.name == format!("Action {n}")) {
                    n = n.checked_add(1).ok_or_else(|| bad("actions.record", "cannot allocate a unique action name"))?;
                }
                format!("Action {n}")
            }
            Some(Value::String(n)) if !n.is_empty() => n.clone(),
            Some(Value::String(_)) => return Err(bad("actions.record", "\"name\" is empty")),
            Some(_) => return Err(bad("actions.record", "\"name\" must be a string")),
        };
        if s.actions.list.iter().any(|action| action.name == name) {
            return Err(bad("actions.record", "an action with this name already exists"));
        }
        s.actions.list.push(Action { name, steps: Vec::new() });
        s.actions.touch();
        s.actions.list.len() - 1
    };
    s.actions.recording = Some((s.journal.len(), idx));
    s.actions.portable.clear();
    let name = s.actions.list.get(idx).map(|a| a.name.clone()).unwrap_or_default();
    Ok(json!({"action": name, "index": idx}))
}

/// Replayable journal entries not yet committed to this action while recording, in their
/// document-independent form with typing merged.
pub fn pending_steps(s: &Session, action: usize) -> Vec<(String, Value)> {
    let from = s.actions.recording.filter(|(_, i)| *i == action).map_or(s.journal.len(), |(from, _)| from);
    let mut out = Vec::new();
    for (i, (id, p)) in s.journal.iter().enumerate().skip(from) {
        if !replayable(id) {
            continue;
        }
        let p = s.actions.portable.iter().find(|(j, _)| *j == i).map_or_else(|| p.clone(), |(_, v)| v.clone());
        push_merged(&mut out, (id.clone(), p));
    }
    out
}

/// Key of a layer reference in a recorded step.
const LAYER_REF: &str = "$layer";

/// The document-independent form of a step about to run, or `None` when it is already
/// portable. Called before the command runs, so "active" means the layer it targets now.
pub(crate) fn portable(s: &Session, p: &Value) -> Option<Value> {
    let obj = p.as_object()?;
    let st = s.active();
    let to_ref = |v: &Value| -> Option<Value> {
        let n = v.as_u64()?;
        let st = st?;
        if st.active_layer.map(|l| l.0) == Some(n) {
            return Some(json!({ LAYER_REF: "active" }));
        }
        Some(json!({ LAYER_REF: { "name": st.doc.layer(photocraft_doc::LayerId(n))?.name } }))
    };
    let mut out = obj.clone();
    let mut changed = out.remove("coalesce").is_some();
    if let Some(r) = obj.get("layer").and_then(to_ref) {
        out.insert("layer".into(), r);
        changed = true;
    }
    if let Some(Value::Array(ids)) = obj.get("layers")
        && !ids.is_empty()
        && let Some(refs) = ids.iter().map(to_ref).collect::<Option<Vec<_>>>()
    {
        out.insert("layers".into(), Value::Array(refs));
        changed = true;
    }
    changed.then_some(Value::Object(out))
}

/// Replaces the layer references of a recorded step (see [`portable`]) with the ids they
/// name in the active document. Params without references pass through unchanged.
pub(crate) fn resolve_refs(s: &Session, cmd: &str, p: Value) -> Result<Value> {
    let Value::Object(mut obj) = p else { return Ok(p) };
    let is_ref = |v: &Value| v.get(LAYER_REF).is_some();
    let has = obj.get("layer").is_some_and(is_ref) || obj.get("layers").and_then(Value::as_array).is_some_and(|a| a.iter().any(is_ref));
    if !has {
        return Ok(Value::Object(obj));
    }
    let one = |v: &Value| -> Result<Value> {
        let Some(r) = v.get(LAYER_REF) else { return Ok(v.clone()) };
        let st = s.active().ok_or_else(|| bad(cmd, "no document is open"))?;
        if r.as_str() == Some("active") {
            return st.active_layer.map(|l| json!(l.0)).ok_or_else(|| bad(cmd, "no active layer"));
        }
        let name = r.get("name").and_then(Value::as_str).ok_or_else(|| bad(cmd, "a layer reference needs \"active\" or a \"name\""))?;
        // Topmost layer with that name, as the Layers panel lists it.
        st.doc.walk().iter().rev().find(|(_, _, l)| l.name == name).map(|(_, _, l)| json!(l.id.0)).ok_or_else(|| bad(cmd, format!("no layer named “{name}”")))
    };
    if let Some(v) = obj.get("layer") {
        let r = one(v)?;
        obj.insert("layer".into(), r);
    }
    if let Some(Value::Array(a)) = obj.get("layers") {
        let r = a.iter().map(one).collect::<Result<Vec<_>>>()?;
        obj.insert("layers".into(), Value::Array(r));
    }
    Ok(Value::Object(obj))
}

/// Appends a step, merging a keystroke into the previous `type.edit` when it only edits the
/// text that one inserted (typing, Backspace inside it), so typed text is one step.
fn push_merged(steps: &mut Vec<(String, Value)>, step: (String, Value)) {
    if let Some(last) = steps.last_mut()
        && let Some(merged) = merge_typing(last, &step)
    {
        last.1 = merged;
        return;
    }
    steps.push(step);
}

fn merge_typing(a: &(String, Value), b: &(String, Value)) -> Option<Value> {
    if a.0 != "type.edit" || b.0 != "type.edit" {
        return None;
    }
    let (pa, pb) = (a.1.as_object()?, b.1.as_object()?);
    let only = |o: &serde_json::Map<String, Value>| o.keys().all(|k| k == "layer" || k == "replace");
    if !only(pa) || !only(pb) || pa.get("layer") != pb.get("layer") {
        return None;
    }
    let part = |o: &serde_json::Map<String, Value>| -> Option<(u64, u64, String)> {
        let r = o.get("replace")?;
        Some((r.get("start")?.as_u64()?, r.get("end")?.as_u64()?, r.get("text")?.as_str()?.to_string()))
    };
    let ((s1, e1, t1), (s2, e2, t2)) = (part(pa)?, part(pb)?);
    let chars: Vec<char> = t1.chars().collect();
    let end1 = s1.checked_add(chars.len() as u64)?;
    if s2 < s1 || e2 < s2 || e2 > end1 {
        return None;
    }
    let (rs, re) = (usize::try_from(s2 - s1).ok()?, usize::try_from(e2 - s1).ok()?);
    let text: String = chars.get(..rs)?.iter().chain(t2.chars().collect::<Vec<_>>().iter()).chain(chars.get(re..)?.iter()).collect();
    let mut out = pa.clone();
    out.insert("replace".into(), json!({"start": s1, "end": e1, "text": text}));
    Some(Value::Object(out))
}

fn flush_recording(s: &mut Session) -> Result<()> {
    let Some((_, idx)) = s.actions.recording else { return Ok(()) };
    if idx >= s.actions.list.len() {
        return Err(bad("actions.stop", "the action being recorded no longer exists"));
    }
    let steps = pending_steps(s, idx);
    s.actions.recording = Some((s.journal.len(), idx));
    s.actions.portable.clear();
    if !steps.is_empty() {
        s.actions.list[idx].steps.extend(steps);
        s.actions.touch();
    }
    Ok(())
}

fn stop(s: &mut Session, _p: &Value) -> Result<Value> {
    not_playing(s, "actions.stop")?;
    let Some((_, idx)) = s.actions.recording else {
        return Err(bad("actions.stop", "not recording"));
    };
    flush_recording(s)?;
    s.actions.recording = None;
    let action = &s.actions.list[idx];
    Ok(json!({"action": action.name, "steps": action.steps.len()}))
}

fn run_recorded(s: &mut Session, steps: &[(String, Value)], from: usize) -> (u64, Option<Value>) {
    let mut ran = 0u64;
    let mut failed = None;
    for (i, (id, params)) in steps.iter().enumerate().skip(from) {
        if let Some(auth) = s.authorize
            && let Err(e) = auth(id, params)
        {
            failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
            break;
        }
        match s.execute(id, params.clone()) {
            Ok(value) => {
                if let Some(error) = nested_failure(id, &value) {
                    failed = Some(json!({"step": i, "id": id, "error": error}));
                    break;
                }
                ran += 1;
            }
            Err(e) => {
                failed = Some(json!({"step": i, "id": id, "error": e.to_string()}));
                break;
            }
        }
    }
    (ran, failed)
}

/// Suspend recording while a called action executes. Keep its commands in the journal
/// for diagnostics, but record just the named call in the parent action.
pub fn begin_playback(s: &mut Session, action: &Action) -> Result<Option<(usize, usize)>> {
    flush_recording(s)?;
    let recording = s.actions.recording.take();
    s.actions.playing = s.actions.playing.saturating_add(1);
    s.actions.playback_stack.push(action.name.clone());
    Ok(recording)
}

pub fn finish_playback(s: &mut Session, recording: Option<(usize, usize)>, action: &Action, from: usize) {
    s.actions.playback_stack.pop();
    s.actions.playing = s.actions.playing.saturating_sub(1);
    if let Some((_, idx)) = recording {
        s.actions.recording = Some((s.journal.len(), idx));
        let mut params = json!({"action": action.name});
        if from > 0 {
            params["from"] = json!(from);
        }
        s.journal.push(("actions.play".into(), params));
    }
}

/// Nested playback returns a structured failure even though the command itself succeeded.
/// Promote it to the caller's failed step so no later parent steps run after a child fails.
pub fn nested_failure(id: &str, value: &Value) -> Option<String> {
    (id == "actions.play").then(|| value.get("failed")).flatten().map(|failure| {
        format!(
            "action `{}` step {} failed: {}",
            value.get("action").and_then(Value::as_str).unwrap_or(""),
            failure.get("step").and_then(Value::as_u64).unwrap_or(0).saturating_add(1),
            failure.get("error").and_then(Value::as_str).unwrap_or("unknown error")
        )
    })
}

fn play(s: &mut Session, p: &Value) -> Result<Value> {
    let (action, from) = playback_plan(s, p)?;
    let recording = begin_playback(s, &action)?;
    let (ran, failed) = run_recorded(s, &action.steps, from);
    finish_playback(s, recording, &action, from);
    Ok(match failed {
        Some(f) => json!({"action": action.name, "ran": ran, "failed": f}),
        None => json!({"action": action.name, "ran": ran}),
    })
}

fn not_playing(s: &Session, cmd: &str) -> Result<()> {
    if s.actions.playing > 0 { Err(bad(cmd, "cannot edit actions during playback")) } else { Ok(()) }
}

/// Move to a final zero-based index, either in an action's steps or in the action list.
fn move_item(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.move")?;
    let idx = resolve(&s.actions.list, p, "actions.move")?;
    let index = |key: &str| -> Result<usize> {
        p.get(key)
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| bad("actions.move", format!("{key} must be a non-negative integer")))
    };
    let to = index("to")?;
    if p.get("step").is_some() {
        let step = index("step")?;
        let len = s.actions.list[idx].steps.len().saturating_add(pending_steps(s, idx).len());
        if step >= len || to >= len {
            return Err(bad("actions.move", "step or destination is out of range"));
        }
        if s.actions.recording.is_some_and(|(_, i)| i == idx) {
            flush_recording(s)?;
        }
        let action = &mut s.actions.list[idx];
        let moved = action.steps.remove(step);
        action.steps.insert(to, moved);
        let result = json!({"action": action.name, "step": to});
        s.actions.touch();
        Ok(result)
    } else {
        if to >= s.actions.list.len() {
            return Err(bad("actions.move", "destination is out of range"));
        }
        let moved = s.actions.list.remove(idx);
        let result = json!({"action": moved.name, "index": to});
        s.actions.list.insert(to, moved);
        if let Some((_, recording)) = s.actions.recording.as_mut() {
            *recording = moved_index(*recording, idx, to);
        }
        s.actions.touch();
        Ok(result)
    }
}

/// Follow a selected/recording row when a list item is moved.
pub fn moved_index(index: usize, from: usize, to: usize) -> usize {
    if index == from {
        to
    } else if from < index && index <= to {
        index - 1
    } else if to <= index && index < from {
        index + 1
    } else {
        index
    }
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    not_playing(s, "actions.delete")?;
    let idx = resolve(&s.actions.list, p, "actions.delete")?;
    if let Some(step) = p.get("step") {
        let step = step.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad("actions.delete", "step must be a non-negative integer"))?;
        let len = s.actions.list[idx].steps.len().saturating_add(pending_steps(s, idx).len());
        if step >= len {
            return Err(bad("actions.delete", "step is out of range"));
        }
        // Commit pending entries before editing the action; never erase the command
        // journal or undo the edit that produced the deleted recording step.
        if s.actions.recording.is_some_and(|(_, i)| i == idx) {
            flush_recording(s)?;
        }
        let action = &mut s.actions.list[idx];
        action.steps.remove(step);
        let result = json!({"action": action.name, "deletedStep": step, "steps": action.steps.len()});
        s.actions.touch();
        return Ok(result);
    }
    if s.actions.recording.is_some() {
        return Err(bad("actions.delete", "stop recording before deleting an action"));
    }
    let name = s.actions.list.remove(idx).name;
    s.actions.touch();
    Ok(json!({"deleted": name}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "actions.move",
            label: "Move Action or Step",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "step":index?, "to":index}. Move a step within its action, or the action itself. to is the final zero-based index. Pending recorded steps can be moved."##,
            enabled: always,
            run: move_item,
            journal: false,
        },
        CommandSpec {
            id: "actions.list",
            label: "List Actions",
            menu: &[],
            shortcut: None,
            params: "{} → {actions:[{name, steps:count}], recording:name|null}",
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "actions.get",
            label: "Get Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index} → {name, steps:[[id, params]…]} (the shape file.automate.batch and droplets take)"##,
            enabled: always,
            run: get,
            journal: false,
        },
        CommandSpec {
            id: "actions.record",
            label: "Record Action",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str? (new action, default "Action N")} or {"action":name|index} (append) → {action, index}"##,
            enabled: always,
            run: record,
            journal: false,
        },
        CommandSpec {
            id: "actions.stop",
            label: "Stop Recording",
            menu: &[],
            shortcut: None,
            params: "{} → {action, steps:count} (steps recorded since actions.record, queries and action-editing commands omitted)",
            enabled: always,
            run: stop,
            journal: false,
        },
        CommandSpec {
            id: "actions.play",
            label: "Play Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "from":step?} → {action, ran, failed?:{step, id, error}}. step and from are 0-based. Stops at the first error (the command still returns ok, with failed set) and leaves one history step per step that ran. Nested calls are supported up to 16 levels; cycles are rejected. Recording stores a named call rather than its expanded commands. Each step is checked with Session::authorize when one is installed."##,
            enabled: always,
            run: play,
            journal: false,
        },
        CommandSpec {
            id: "actions.delete",
            label: "Delete Action",
            menu: &[],
            shortcut: None,
            params: r##"{"action":name|index, "step":index?} → {deleted:name} or {action, deletedStep, steps:count}. step is 0-based, including pending recorded steps. Whole-action deletion is refused while recording."##,
            enabled: always,
            run: delete,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pixel(s: &Session, layer: usize, x: i32, y: i32) -> Vec<f32> {
        s.active().unwrap().doc.layers[layer].surface().unwrap().pixel(x, y)
    }

    fn record_red(s: &mut Session, depth: u32) {
        s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
        s.execute("actions.record", json!({"name": "Red"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let _ = s.execute("document.pixel", json!({"x": 1, "y": 1}));
        let _ = s.execute("actions.list", json!({}));
        s.execute("actions.stop", json!({})).unwrap();
    }

    fn translated_square(s: &mut Session, depth: u32) {
        s.execute("file.new", json!({"width": 20, "height": 8, "depth": depth, "background": "transparent"})).unwrap();
        s.edit("red square", |doc, _| {
            doc.layers[0].surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 2, 2, 4), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 4, 0], "interpolation": "nearest"})).unwrap();
        assert_eq!(pixel(s, 0, 4, 2), vec![1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn record_transform_again_replays_one_concrete_transform_at_each_depth() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            translated_square(&mut s, depth);
            let from = s.journal.len();
            s.execute("actions.record", json!({"name": "Again"})).unwrap();
            s.execute("edit.transform.again", json!({})).unwrap();
            let ids: Vec<&str> = s.journal.iter().skip(from).map(|(id, _)| id.as_str()).collect();
            assert_eq!(ids, ["edit.transform", "edit.transform.again"], "both commands stay in the journal during recording");
            let live_count = s.journal.iter().skip(from).filter(|(id, _)| replayable(id)).count();
            s.execute("actions.stop", json!({})).unwrap();
            let recorded = s.execute("actions.get", json!({"action": "Again"})).unwrap();
            let steps = recorded["steps"].as_array().unwrap();
            assert_eq!(steps.len(), 1, "depth {depth}: record the concrete transform once");
            assert_eq!(live_count, 1, "depth {depth}: the live Actions count agrees");
            assert_eq!(steps[0][0], "edit.transform");
            assert_eq!(steps[0][1]["matrix"], json!([1, 0, 0, 1, 4, 0]));
            assert_eq!(steps[0][1]["interpolation"], "nearest");

            translated_square(&mut s, depth);
            let before = s.active().unwrap().history.past_len();
            let played = s.execute("actions.play", json!({"action": "Again"})).unwrap();
            assert!(played.get("failed").is_none(), "depth {depth}: {played}");
            assert_eq!(played["ran"], 1, "depth {depth}: apply the recorded transform once");
            assert_eq!(s.active().unwrap().history.past_len(), before + 1, "depth {depth}: one undo step");
            assert_eq!(pixel(&s, 0, 8, 2), vec![1.0, 0.0, 0.0, 1.0], "depth {depth}: moved four pixels");
            assert_eq!(pixel(&s, 0, 12, 2), vec![0.0; 4], "depth {depth}: no second transform");
            assert!(s.undo());
            assert_eq!(pixel(&s, 0, 4, 2), vec![1.0, 0.0, 0.0, 1.0]);
            assert_eq!(pixel(&s, 0, 8, 2), vec![0.0; 4]);
            assert!(s.redo());
            assert_eq!(pixel(&s, 0, 8, 2), vec![1.0, 0.0, 0.0, 1.0]);
            assert_eq!(pixel(&s, 0, 4, 2), vec![0.0; 4]);
        }
    }

    #[test]
    fn record_explicit_transform_keeps_one_replayable_step() {
        let mut s = Session::new();
        translated_square(&mut s, 8);
        s.execute("actions.record", json!({"name": "Move"})).unwrap();
        s.execute("edit.transform", json!({"matrix": [1, 0, 0, 1, 4, 0], "interpolation": "nearest"})).unwrap();
        assert_eq!(s.execute("actions.stop", json!({})).unwrap()["steps"], 1);
        let recorded = s.execute("actions.get", json!({"action": "Move"})).unwrap();
        assert_eq!(recorded["steps"][0][0], "edit.transform");

        translated_square(&mut s, 8);
        let before = s.active().unwrap().history.past_len();
        let played = s.execute("actions.play", json!({"action": "Move"})).unwrap();
        assert!(played.get("failed").is_none(), "{played}");
        assert_eq!(played["ran"], 1);
        assert_eq!(s.active().unwrap().history.past_len(), before + 1);
        assert_eq!(pixel(&s, 0, 8, 2), vec![1.0, 0.0, 0.0, 1.0]);
        assert_eq!(pixel(&s, 0, 12, 2), vec![0.0; 4]);
    }

    #[test]
    fn record_keeps_only_replayable_steps() {
        let mut s = Session::new();
        record_red(&mut s, 8);
        let got = s.execute("actions.get", json!({"action": "Red"})).unwrap();
        let ids: Vec<&str> = got["steps"].as_array().unwrap().iter().map(|st| st[0].as_str().unwrap()).collect();
        assert_eq!(ids, ["layer.new.layer", "select.rect", "edit.fill"]);
        assert!(s.journal.iter().all(|(id, _)| !id.starts_with("actions.")));
        let listed = s.execute("actions.list", json!({})).unwrap();
        assert_eq!(listed["actions"][0]["steps"], 3);
        assert!(listed["recording"].is_null());
    }

    fn layer_named<'a>(s: &'a Session, name: &str) -> &'a photocraft_doc::Layer {
        s.active().unwrap().doc.walk().into_iter().map(|(_, _, l)| l).find(|l| l.name == name).unwrap()
    }

    #[test]
    fn recorded_layer_ids_replay_by_reference_on_another_document() {
        // AUTO-219-1: the active layer is implicit and other layers are named, so the action
        // targets the same layers on a document whose layer ids differ.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Shade"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
        let (shade, top) = (layer_named(&s, "Shade").id.0, layer_named(&s, "Top").id.0);
        s.execute("actions.record", json!({"name": "Fade"})).unwrap();
        s.execute("layer.setProps", json!({"layer": shade, "opacity": 0.5})).unwrap();
        s.execute("layer.setProps", json!({"layer": top, "opacity": 0.25})).unwrap();
        s.execute("layer.setProps", json!({"layers": [shade, top], "visible": true})).ok();
        s.execute("actions.stop", json!({})).unwrap();
        let steps = &s.actions.list[0].steps;
        assert_eq!(steps[0].1, json!({"layer": {"$layer": {"name": "Shade"}}, "opacity": 0.5}));
        assert_eq!(steps[1].1, json!({"layer": {"$layer": "active"}, "opacity": 0.25}));
        assert!(!serde_json::to_string(steps).unwrap().contains(&format!(":{shade}")), "{steps:?}");

        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        for name in ["Extra", "Shade", "Other", "Top", "Last"] {
            s.execute("layer.new.layer", json!({"name": name})).unwrap();
        }
        assert_ne!(layer_named(&s, "Shade").id.0, shade, "the ids differ on the second document");
        let r = s.execute("actions.play", json!({"action": "Fade"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        assert_eq!(layer_named(&s, "Shade").opacity, 0.5);
        assert_eq!(layer_named(&s, "Last").opacity, 0.25, "the active layer, whatever its name");
        assert_eq!(layer_named(&s, "Top").opacity, 1.0);
        assert_eq!(layer_named(&s, "Extra").opacity, 1.0);
    }

    #[test]
    fn a_missing_named_layer_fails_the_step_cleanly() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let err = s.execute("layer.setProps", json!({"layer": {"$layer": {"name": "Nope"}}, "opacity": 0.5})).unwrap_err();
        assert!(err.to_string().contains("Nope"), "{err}");
        for bad in [json!({"$layer": 7}), json!({"$layer": {"name": 3}}), json!({"$layer": null})] {
            assert!(s.execute("layer.setProps", json!({"layer": bad, "opacity": 0.5})).is_err());
        }
        // Without a document an active reference is an error, not a panic.
        let mut empty = Session::new();
        assert!(empty.execute("layer.setProps", json!({"layer": {"$layer": "active"}})).is_err());
    }

    #[test]
    fn typed_keystrokes_record_as_one_step() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 64, "height": 32})).unwrap();
        s.execute("actions.record", json!({"name": "Type"})).unwrap();
        let id = s.execute("type.create", json!({"x": 4, "y": 20, "text": "H"})).unwrap()["layer"].as_u64().unwrap();
        for (start, end, text) in [(1, 1, "i"), (2, 2, "!!"), (3, 4, "")] {
            s.execute("type.edit", json!({"layer": id, "replace": {"start": start, "end": end, "text": text}, "coalesce": "typing-1"})).unwrap();
        }
        // A style change is its own step, and typing after it starts a new one.
        s.execute("type.edit", json!({"layer": id, "antialias": "crisp"})).unwrap();
        s.execute("type.edit", json!({"layer": id, "replace": {"start": 3, "end": 3, "text": "?"}, "coalesce": "typing-2"})).unwrap();
        assert_eq!(pending_steps(&s, 0).len(), 4, "the panel shows the merged steps while recording");
        s.execute("actions.stop", json!({})).unwrap();
        let steps = &s.actions.list[0].steps;
        let ids: Vec<&str> = steps.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["type.create", "type.edit", "type.edit", "type.edit"]);
        assert_eq!(steps[1].1, json!({"layer": {"$layer": "active"}, "replace": {"start": 1, "end": 1, "text": "i!"}}));
        assert_eq!(steps[3].1, json!({"layer": {"$layer": "active"}, "replace": {"start": 3, "end": 3, "text": "?"}}));

        s.execute("file.new", json!({"width": 64, "height": 32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let r = s.execute("actions.play", json!({"action": "Type"})).unwrap();
        assert!(r.get("failed").is_none(), "{r}");
        assert_eq!(s.execute("type.info", json!({})).unwrap()["text"], "Hi!?");
    }

    #[test]
    fn merge_typing_only_merges_edits_inside_the_inserted_text() {
        let edit = |s: u64, e: u64, t: &str| ("type.edit".to_string(), json!({"layer": 1, "replace": {"start": s, "end": e, "text": t}}));
        assert_eq!(merge_typing(&edit(0, 0, "ab"), &edit(2, 2, "c")), Some(edit(0, 0, "abc").1));
        assert_eq!(merge_typing(&edit(5, 7, "xy"), &edit(5, 6, "")), Some(edit(5, 7, "y").1));
        assert_eq!(merge_typing(&edit(0, 0, "ab"), &edit(3, 3, "c")), None, "past the inserted text");
        assert_eq!(merge_typing(&edit(4, 4, "ab"), &edit(3, 3, "c")), None, "before it");
        assert_eq!(merge_typing(&edit(0, 0, "ab"), &edit(2, 1, "c")), None, "a reversed range");
        let other = ("type.edit".to_string(), json!({"layer": 2, "replace": {"start": 2, "end": 2, "text": "c"}}));
        assert_eq!(merge_typing(&edit(0, 0, "ab"), &other), None, "another layer");
        assert_eq!(merge_typing(&edit(0, 0, "é✓"), &edit(1, 2, "e")), Some(edit(0, 0, "ée").1), "char offsets, not bytes");
        assert_eq!(merge_typing(&edit(u64::MAX, u64::MAX, "a"), &edit(u64::MAX, u64::MAX, "b")), None, "overflow");
    }

    #[test]
    fn play_on_another_document_matches_pixels_at_each_depth() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            record_red(&mut s, depth);
            s.execute("file.new", json!({"width": 40, "height": 40, "depth": depth})).unwrap();
            let before = s.active().unwrap().history.past_len();
            let r = s.execute("actions.play", json!({"action": 0})).unwrap();
            assert!(r.get("failed").is_none(), "depth {depth}: {r}");
            assert_eq!(r["ran"], 3, "depth {depth}");
            assert_eq!(s.active().unwrap().doc.layers.len(), 2, "depth {depth}");
            assert_eq!(s.active().unwrap().history.past_len(), before + 3, "depth {depth}: one history step per played step");
            let px = pixel(&s, 1, 5, 5);
            assert!(px.iter().zip([1.0, 0.0, 0.0, 1.0]).all(|(a, b)| (a - b).abs() < 1.0e-5), "depth {depth}: {px:?}");
            assert_eq!(s.actions.playing, 0);
        }
    }

    #[test]
    fn play_stops_at_the_failing_step_and_reports_it() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        s.execute("actions.record", json!({})).unwrap();
        s.execute("layer.new.layer", json!({"name": "One"})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        s.actions.list[0].steps.push(("not.a.command".into(), json!({})));
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let r = s.execute("actions.play", json!({"action": "Action 1"})).unwrap();
        assert_eq!(r["ran"], 1);
        assert_eq!(r["failed"]["step"], 1);
        assert_eq!(r["failed"]["id"], "not.a.command");
        assert!(r["failed"]["error"].as_str().unwrap().contains("unknown command"));
        assert_eq!(s.active().unwrap().doc.layers.len(), 2, "the step before the failure is kept");
        assert_eq!(s.actions.playing, 0);
        // A later play still runs.
        assert_eq!(s.execute("actions.play", json!({"action": 0, "from": 1})).unwrap()["ran"], 0);
    }

    #[test]
    fn play_refuses_recursion() {
        let mut s = Session::new();
        s.actions.list.push(Action { name: "Loop".into(), steps: vec![("actions.play".into(), json!({"action": "Loop"}))] });
        let r = s.execute("actions.play", json!({"action": "Loop"})).unwrap();
        assert_eq!(r["ran"], 0);
        assert_eq!(r["failed"]["id"], "actions.play");
        assert!(r["failed"]["error"].as_str().unwrap().contains("recursive action"), "{r}");
        assert_eq!(s.actions.playing, 0);
    }

    #[test]
    fn play_checks_each_step_with_the_authorize_hook() {
        fn deny_file(id: &str, _: &Value) -> Result<()> {
            if id.starts_with("file.") { Err(EngineError::Other(format!("automation command `{id}` is disabled"))) } else { Ok(()) }
        }
        let mut s = Session::new();
        s.authorize = Some(deny_file);
        s.actions
            .list
            .push(Action { name: "Open".into(), steps: vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))] });
        let r = s.execute("actions.play", json!({"action": "Open"})).unwrap();
        assert_eq!(r["ran"], 0);
        assert_eq!(r["failed"]["step"], 0);
        assert_eq!(r["failed"]["id"], "file.open");
        assert!(r["failed"]["error"].as_str().unwrap().contains("disabled"), "{r}");
        assert!(s.documents().is_empty(), "the refused step does not open a document");
    }

    #[test]
    fn append_delete_and_from_step() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
        let started = s.execute("actions.record", json!({"name": "Grow"})).unwrap();
        assert_eq!(started["index"], 0);
        s.execute("layer.new.layer", json!({"name": "One"})).unwrap();
        assert!(s.execute("actions.record", json!({})).is_err(), "already recording");
        assert!(s.execute("actions.delete", json!({"action": 0})).is_err(), "delete while recording");
        let stopped = s.execute("actions.stop", json!({})).unwrap();
        assert_eq!(stopped["steps"], 1);
        s.execute("actions.record", json!({"action": "Grow"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Two"})).unwrap();
        assert_eq!(s.execute("actions.stop", json!({})).unwrap()["steps"], 2);
        assert!(s.execute("actions.stop", json!({})).is_err(), "not recording");
        let before = s.active().unwrap().doc.layers.len();
        let r = s.execute("actions.play", json!({"action": "Grow", "from": 1})).unwrap();
        assert_eq!(r["ran"], 1);
        assert_eq!(s.active().unwrap().doc.layers.len(), before + 1);
        assert_eq!(s.execute("actions.delete", json!({"action": "Grow"})).unwrap()["deleted"], "Grow");
        assert!(s.actions.list.is_empty());
        assert!(s.execute("actions.get", json!({"action": "Grow"})).is_err());
    }

    #[test]
    fn delete_one_step_while_recording_keeps_journal_and_remaining_steps() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("actions.record", json!({"name": "Edit"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Kept"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Removed"})).unwrap();
        let journal = s.journal.clone();
        let rev = s.actions.rev;
        let before = s.actions.clone();
        for step in [json!(-1), json!(2), json!(0.5), json!(null), json!("0"), json!(u64::MAX)] {
            assert!(s.execute("actions.delete", json!({"action": 0, "step": step})).is_err());
            assert_eq!(s.actions, before, "invalid deletion is atomic");
        }
        assert_eq!(pending_steps(&s, 0).len(), 2);
        let result = s.execute("actions.delete", json!({"action": 0, "step": 1})).unwrap();
        assert_eq!(result["steps"], 1);
        assert_eq!(s.journal, journal, "deleting a recording does not erase executed commands");
        assert_eq!(s.active().unwrap().doc.layers.len(), 3, "it does not undo document edits");
        assert!(s.actions.recording.is_some());
        assert!(s.actions.rev > rev);
        s.execute("layer.new.layer", json!({"name": "After"})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        assert_eq!(s.actions.list[0].steps.iter().map(|(_, p)| p["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Kept", "After"]);
        s.execute("actions.delete", json!({"action": "Edit", "step": 0})).unwrap();
        assert_eq!(s.actions.list[0].steps[0].1["name"], "After");
        let saved = serde_json::to_string(&s.actions.list).unwrap();
        s.actions.list = serde_json::from_str(&saved).unwrap();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        assert_eq!(s.execute("actions.play", json!({"action": 0})).unwrap()["ran"], 1);
        assert_eq!(s.active().unwrap().doc.layers[1].name, "After");
        s.execute("actions.delete", json!({"action": 0, "step": 0})).unwrap();
        assert_eq!(s.actions.list.len(), 1, "an empty action still exists");
        assert!(s.actions.list[0].steps.is_empty());
    }

    #[test]
    fn nested_call_records_one_named_step_and_tracks_child_edits() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        s.actions.list.push(Action { name: "Child".into(), steps: vec![("layer.new.layer".into(), json!({"name": "Child layer"}))] });
        s.execute("actions.record", json!({"name": "Parent"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Before"})).unwrap();
        s.execute("actions.play", json!({"action": 0})).unwrap();
        assert_eq!(pending_steps(&s, 1), [("actions.play".into(), json!({"action": "Child"}))]);
        s.execute("layer.new.layer", json!({"name": "After"})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        assert_eq!(s.actions.list[1].steps.iter().map(|st| st.0.as_str()).collect::<Vec<_>>(), ["layer.new.layer", "actions.play", "layer.new.layer"]);
        s.actions.list[0].steps.push(("layer.new.layer".into(), json!({"name": "Added later"})));
        s.execute("actions.move", json!({"action": "Child", "to": 1})).unwrap();
        let saved = serde_json::to_string(&s.actions.list).unwrap();
        s.actions.list = serde_json::from_str(&saved).unwrap();
        s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        let result = s.execute("actions.play", json!({"action": "Parent"})).unwrap();
        assert!(result.get("failed").is_none(), "{result}");
        assert_eq!(
            s.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.as_str()).collect::<Vec<_>>(),
            ["Before", "Child layer", "Added later", "After"]
        );
        assert!(s.actions.playback_stack.is_empty());
    }

    #[test]
    fn nested_failure_cycles_depth_and_authorization_stop_parent() {
        let mut s = Session::new();
        s.actions.list.push(Action {
            name: "A".into(),
            steps: vec![("actions.play".into(), json!({"action": "B"})), ("file.new".into(), json!({"width": 2,"height": 2}))],
        });
        s.actions.list.push(Action { name: "B".into(), steps: vec![("actions.play".into(), json!({"action": "A"}))] });
        let r = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert!(r["failed"]["error"].as_str().unwrap().contains("recursive"));
        assert!(s.documents().is_empty());
        s.actions.list[1].steps = vec![("file.new".into(), json!({"width": 2,"height": 2}))];
        s.authorize = Some(|id, _| if id == "file.new" { Err(EngineError::Other("denied nested file".into())) } else { Ok(()) });
        let r = s.execute("actions.play", json!({"action": "A"})).unwrap();
        assert!(r["failed"]["error"].as_str().unwrap().contains("denied nested file"));
        assert!(s.documents().is_empty());
        s.authorize = None;
        s.actions.list.clear();
        for i in 0..17 {
            s.actions.list.push(Action {
                name: format!("N{i}"),
                steps: if i < 16 { vec![("actions.play".into(), json!({"action":format!("N{}", i + 1)}))] } else { vec![] },
            });
        }
        let r = s.execute("actions.play", json!({"action": "N0"})).unwrap();
        assert!(r["failed"]["error"].as_str().unwrap().contains("16 levels"));
        assert_eq!(s.actions.playing, 0);
        assert!(s.actions.playback_stack.is_empty());
        s.actions.list[15].steps.clear();
        assert!(s.execute("actions.play", json!({"action": "N0"})).unwrap().get("failed").is_none());
    }

    #[test]
    fn move_pending_steps_is_atomic_persists_and_changes_playback_order() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        s.execute("actions.record", json!({"name": "Order"})).unwrap();
        for name in ["One", "Two", "Three"] {
            s.execute("layer.new.layer", json!({"name": name})).unwrap();
        }
        let before = s.actions.clone();
        for params in [
            json!({"action":0,"step":0,"to":3}),
            json!({"action":0,"step":99,"to":0}),
            json!({"action":0,"step":0.5,"to":0}),
            json!({"action":0,"to":-1}),
            json!({"action":0,"to":null}),
        ] {
            assert!(s.execute("actions.move", params).is_err());
            assert_eq!(s.actions, before);
        }
        let journal = s.journal.clone();
        s.execute("actions.move", json!({"action":0,"step":0,"to":2})).unwrap();
        s.execute("actions.move", json!({"action":0,"step":1,"to":0})).unwrap();
        assert_eq!(s.journal, journal);
        s.execute("layer.new.layer", json!({"name":"Four"})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        assert_eq!(s.actions.list[0].steps.iter().map(|st| st.1["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Three", "Two", "One", "Four"]);
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        s.execute("actions.play", json!({"action":0})).unwrap();
        assert_eq!(s.active().unwrap().doc.layers.iter().skip(1).map(|l| l.name.as_str()).collect::<Vec<_>>(), ["Three", "Two", "One", "Four"]);
    }

    #[test]
    fn new_names_do_not_collide_with_named_calls() {
        let mut s = Session::new();
        s.actions.list.push(Action { name: "Action 2".into(), steps: vec![] });
        assert_eq!(s.execute("actions.record", json!({})).unwrap()["action"], "Action 3");
        s.execute("actions.stop", json!({})).unwrap();
        assert!(s.execute("actions.record", json!({"name":"Action 2"})).is_err());
        assert!(s.actions.recording.is_none());
    }

    #[test]
    fn moving_action_keeps_recording_attached_to_it() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":4,"height":4})).unwrap();
        for name in ["A", "B", "C"] {
            s.actions.list.push(Action { name: name.into(), steps: vec![] });
        }
        s.execute("actions.record", json!({"action":"B"})).unwrap();
        s.execute("layer.new.layer", json!({"name":"before"})).unwrap();
        s.execute("actions.move", json!({"action":"B","to":2})).unwrap();
        s.execute("actions.move", json!({"action":"A","to":2})).unwrap();
        s.execute("layer.new.layer", json!({"name":"after"})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        assert_eq!(s.actions.list[1].name, "B");
        assert_eq!(s.actions.list[1].steps.len(), 2);
    }

    #[test]
    fn bad_params_do_not_panic() {
        let mut s = Session::new();
        let junk = [
            Value::Null,
            json!([]),
            json!("x"),
            json!(1),
            json!({"action": true}),
            json!({"action": -1}),
            json!({"action": 99}),
            json!({"action": ""}),
            json!({"name": 1}),
            json!({"name": ""}),
            json!({"from": "start"}),
            json!({"from": -3}),
            json!({"action": "missing", "from": 4}),
        ];
        for id in ["actions.list", "actions.get", "actions.record", "actions.stop", "actions.play", "actions.delete", "actions.move"] {
            for p in &junk {
                let _ = s.execute(id, p.clone());
            }
        }
        assert_eq!(s.actions.playing, 0);
        assert!(crate::commands::command_specs().iter().any(|c| c.id == "actions.play" && !c.journal && c.menu.is_empty()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn actions_persist_with_the_preset_store() {
        let dir = std::env::temp_dir().join(format!("pc-actions-{}-persist", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Session::new();
        let warnings = s.attach_preset_store(crate::preset_store::open_dir(&dir));
        assert!(warnings.iter().all(|w| !w.contains("actions")), "{warnings:?}");
        s.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        s.execute("actions.record", json!({"name": "Kept"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("actions.stop", json!({})).unwrap();
        drop(s);

        let mut t = Session::new();
        t.attach_preset_store(crate::preset_store::open_dir(&dir));
        let listed = t.execute("actions.list", json!({})).unwrap();
        assert_eq!(listed["actions"][0]["name"], "Kept");
        assert_eq!(listed["actions"][0]["steps"], 1);
        assert_eq!(t.execute("actions.get", json!({"action": "Kept"})).unwrap()["steps"][0][0], "layer.new.layer");

        // An action created before the store finishes loading is kept, and a different one on
        // disk is merged in.
        let mut early = Session::new();
        early.execute("actions.record", json!({"name": "Early"})).unwrap();
        early.execute("actions.stop", json!({})).unwrap();
        early.attach_preset_store(crate::preset_store::open_dir(&dir));
        let names: Vec<String> = early.actions.list.iter().map(|a| a.name.clone()).collect();
        assert!(names.contains(&"Early".into()) && names.contains(&"Kept".into()), "{names:?}");

        std::fs::write(dir.join("actions.json"), b"{").unwrap();
        let mut bad = Session::new();
        let warnings = bad.attach_preset_store(crate::preset_store::open_dir(&dir));
        assert!(warnings.iter().any(|w| w.contains("actions")), "{warnings:?}");
        assert!(bad.actions.list.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
