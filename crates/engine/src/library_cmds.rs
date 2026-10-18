//! Window › Libraries (`library.*`): named libraries of colours, graphics and patterns that
//! outlive documents and can be used in any of them.
//!
//! - A **colour** sets the foreground (or background) colour.
//! - A **graphic** holds a file's bytes: layers saved from a document (packed like Convert to
//!   Smart Object, as a `.pcraft` file) or an image file added by path. Using one places it as
//!   an embedded smart object (a copy), or, with `"linked": true` on the desktop, as a linked
//!   smart object that refers to the library's own file.
//! - A **pattern** holds a pattern tile; using one adds it to the document's patterns.
//!
//! Items are addressed by `item` (an id unique across libraries, see `library.list`); libraries
//! by `library` (name; the first library when omitted, created as "My Library" when there is
//! none). The libraries persist in the preset store (`libraries.json` plus one content-addressed
//! `lib-<hash>.<ext>` file per graphic or pattern); headless sessions keep them in memory.
//! Changing a library is not a document edit and has no undo, like the other preset panels.

use std::sync::Arc;

use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::preset_store::PresetBackend;
use crate::presets::{always, bad, has_doc, str_param};
use crate::{EngineError, Result, Session};

/// The persisted index.
pub const INDEX: &str = "libraries.json";
/// Largest index read or written.
pub const MAX_INDEX_BYTES: u64 = 16 << 20;
/// Largest graphic or pattern file kept.
pub const MAX_ITEM_BYTES: u64 = 256 << 20;
/// All graphic and pattern bytes together.
pub const MAX_TOTAL_BYTES: u64 = 2 << 30;
/// Most libraries.
pub const MAX_LIBRARIES: usize = 200;
/// Most items across all libraries.
pub const MAX_ITEMS: usize = 20_000;
/// Longest library or item name (characters).
pub const MAX_NAME: usize = 255;
/// The library created when an item is added and there is none.
pub const DEFAULT_LIBRARY: &str = "My Library";
const FORMAT: &str = "photocraft-libraries";

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// RGB in `0..=1` per channel, as the tool colours hold it.
    Color { rgb: [f32; 3] },
    /// A placeable file: `file_name` keeps the extension the importer reads.
    Graphic { file_name: String, bytes: Arc<Vec<u8>>, width: u32, height: u32 },
    /// A pattern tile, stored as a one-layer `.pcraft` document.
    Pattern { bytes: Arc<Vec<u8>>, width: u32, height: u32 },
}

impl Kind {
    pub fn key(&self) -> &'static str {
        match self {
            Kind::Color { .. } => "color",
            Kind::Graphic { .. } => "graphic",
            Kind::Pattern { .. } => "pattern",
        }
    }

    fn bytes(&self) -> Option<&Arc<Vec<u8>>> {
        match self {
            Kind::Color { .. } => None,
            Kind::Graphic { bytes, .. } | Kind::Pattern { bytes, .. } => Some(bytes),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Library {
    pub name: String,
    pub items: Vec<Item>,
}

/// Session state: the libraries and a revision the preset store syncs on.
#[derive(Clone, Debug, Default)]
pub struct Libraries {
    pub list: Vec<Library>,
    /// Bumped on every change.
    pub rev: u64,
    next_id: u64,
}

impl Libraries {
    fn changed(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    fn fresh_id(&mut self) -> u64 {
        let max = self.list.iter().flat_map(|l| l.items.iter().map(|i| i.id)).max().unwrap_or(0);
        self.next_id = self.next_id.max(max).saturating_add(1);
        self.next_id
    }

    fn items(&self) -> usize {
        self.list.iter().map(|l| l.items.len()).sum()
    }

    fn bytes(&self) -> u64 {
        self.list.iter().flat_map(|l| l.items.iter()).filter_map(|i| i.kind.bytes()).map(|b| b.len() as u64).sum()
    }

    /// `(library index, item index)` of item `id`.
    pub fn locate(&self, id: u64) -> Option<(usize, usize)> {
        self.list.iter().enumerate().find_map(|(li, l)| l.items.iter().position(|i| i.id == id).map(|ii| (li, ii)))
    }

    pub fn item(&self, id: u64) -> Option<&Item> {
        let (li, ii) = self.locate(id)?;
        self.list.get(li)?.items.get(ii)
    }
}

fn clip(name: &str) -> String {
    name.trim().chars().filter(|c| !c.is_control()).take(MAX_NAME).collect()
}

fn hex(rgb: [f32; 3]) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(rgb[0]), b(rgb[1]), b(rgb[2]))
}

fn item_json(i: &Item) -> Value {
    let mut v = json!({"id": i.id, "name": i.name, "kind": i.kind.key()});
    match &i.kind {
        Kind::Color { rgb } => {
            v["hex"] = json!(hex(*rgb));
            v["rgb"] = json!(rgb);
        }
        Kind::Graphic { file_name, bytes, width, height } => {
            v["fileName"] = json!(file_name);
            v["width"] = json!(width);
            v["height"] = json!(height);
            v["bytes"] = json!(bytes.len());
        }
        Kind::Pattern { bytes, width, height } => {
            v["width"] = json!(width);
            v["height"] = json!(height);
            v["bytes"] = json!(bytes.len());
        }
    }
    v
}

/// The library a command names (`library`), or the first; `create` makes "My Library" when there
/// is no library at all.
fn library_index(s: &mut Session, p: &Value, cmd: &str, create: bool) -> Result<usize> {
    let libs = &mut s.libraries;
    match str_param(p, "library") {
        Some(name) => libs.list.iter().position(|l| l.name.eq_ignore_ascii_case(name.trim())).ok_or_else(|| bad(cmd, format!("no library named `{name}`"))),
        None if !libs.list.is_empty() => Ok(0),
        None if create => {
            libs.list.push(Library { name: DEFAULT_LIBRARY.into(), items: Vec::new() });
            Ok(0)
        }
        None => Err(bad(cmd, "there is no library")),
    }
}

fn item_param(s: &Session, p: &Value, cmd: &str) -> Result<(usize, usize)> {
    let id = p.get("item").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "`item` (an id from library.list) is required"))?;
    s.libraries.locate(id).ok_or_else(|| bad(cmd, format!("no library item {id}")))
}

/// A unique name within `lib` based on `base`.
fn unique_item_name(lib: &Library, base: &str) -> String {
    let taken = |n: &str| lib.items.iter().any(|i| i.name.eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

/// Adds `kind` to the library `p` names, within the limits.
fn add_item(s: &mut Session, p: &Value, cmd: &str, default_name: &str, kind: Kind) -> Result<Value> {
    if s.libraries.items() >= MAX_ITEMS {
        return Err(bad(cmd, format!("the libraries are full ({MAX_ITEMS} items)")));
    }
    if let Some(b) = kind.bytes() {
        if b.len() as u64 > MAX_ITEM_BYTES {
            return Err(bad(cmd, format!("the item is larger than {} MB", MAX_ITEM_BYTES >> 20)));
        }
        if s.libraries.bytes().saturating_add(b.len() as u64) > MAX_TOTAL_BYTES {
            return Err(bad(cmd, format!("the libraries are full ({} MB)", MAX_TOTAL_BYTES >> 20)));
        }
    }
    let li = library_index(s, p, cmd, true)?;
    let id = s.libraries.fresh_id();
    let base = str_param(p, "name").map(clip).filter(|n| !n.is_empty()).unwrap_or_else(|| default_name.to_string());
    let lib = s.libraries.list.get_mut(li).ok_or_else(|| bad(cmd, "no library"))?;
    let name = unique_item_name(lib, &base);
    let item = Item { id, name, kind };
    let out = item_json(&item);
    lib.items.push(item);
    let library = lib.name.clone();
    s.libraries.changed();
    Ok(json!({"library": library, "item": out}))
}

// ---------- commands ----------

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let libs: Vec<Value> = s.libraries.list.iter().map(|l| json!({"name": l.name, "items": l.items.iter().map(item_json).collect::<Vec<_>>()})).collect();
    Ok(json!({"libraries": libs}))
}

fn new_library(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.new";
    let base = str_param(p, "name").map(clip).filter(|n| !n.is_empty()).unwrap_or_else(|| DEFAULT_LIBRARY.to_string());
    if s.libraries.list.len() >= MAX_LIBRARIES {
        return Err(bad(CMD, format!("at most {MAX_LIBRARIES} libraries")));
    }
    let taken = |n: &str, l: &[Library]| l.iter().any(|x| x.name.eq_ignore_ascii_case(n));
    let name = if taken(&base, &s.libraries.list) { (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n, &s.libraries.list)).unwrap_or(base) } else { base };
    s.libraries.list.push(Library { name: name.clone(), items: Vec::new() });
    s.libraries.changed();
    Ok(json!({"library": name}))
}

fn delete_library(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.delete";
    if str_param(p, "library").is_none() {
        return Err(bad(CMD, "`library` (name) is required"));
    }
    let li = library_index(s, p, CMD, false)?;
    let l = s.libraries.list.remove(li);
    s.libraries.changed();
    Ok(json!({"library": l.name, "deleted": l.items.len()}))
}

fn rename_library(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.rename";
    let li = library_index(s, p, CMD, false)?;
    let name = str_param(p, "name").map(clip).filter(|n| !n.is_empty()).ok_or_else(|| bad(CMD, "`name` is required"))?;
    if s.libraries.list.iter().enumerate().any(|(i, l)| i != li && l.name.eq_ignore_ascii_case(&name)) {
        return Err(bad(CMD, format!("a library named `{name}` already exists")));
    }
    if let Some(l) = s.libraries.list.get_mut(li) {
        l.name = name.clone();
    }
    s.libraries.changed();
    Ok(json!({"library": name}))
}

fn add_color(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.addColor";
    let fg = s.tools.foreground;
    let c = crate::commands::color_param(p, "color", fg);
    let rgb = [c[0], c[1], c[2]].map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    let name = hex(rgb);
    add_item(s, p, CMD, &name, Kind::Color { rgb })
}

/// The layers `p` names (`layers`: ids), or the selected layers; top-level within the selection.
fn layer_ids(s: &Session, p: &Value) -> Vec<LayerId> {
    match p.get("layers").and_then(Value::as_array) {
        Some(a) => a.iter().take(10_000).filter_map(Value::as_u64).map(LayerId).collect(),
        None => crate::layer_multi_cmds::selected(s),
    }
}

fn add_graphic(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.addGraphic";
    let ids = layer_ids(s, p);
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &st.doc;
    let ids = crate::layer_multi_cmds::top_level(doc, &ids);
    let top = *ids.last().ok_or_else(|| bad(CMD, "no layer selected"))?;
    let l = doc.layer(top).ok_or(EngineError::NoLayer(top))?;
    let packed = if ids.len() == 1 {
        crate::smart_cmds::layer_to_smart(doc, l)?
    } else {
        let children = ids.iter().map(|id| doc.layer(*id).cloned().ok_or(EngineError::NoLayer(*id))).collect::<Result<Vec<_>>>()?;
        crate::smart_cmds::layer_to_smart(doc, &Layer::group(l.name.clone(), children))?
    };
    let name = packed.name.clone();
    let LayerContent::Smart(sm) = packed.content else { return Err(bad(CMD, "the layers could not be packed")) };
    let photocraft_doc::SmartSource::Embedded { file_name, bytes } = sm.source else {
        return Err(bad(CMD, "the layers could not be packed"));
    };
    let (width, height) = sm
        .cache
        .as_ref()
        .map(|c| {
            let b = c.content_bounds();
            (b.width(), b.height())
        })
        .unwrap_or((0, 0));
    add_item(s, p, CMD, &name, Kind::Graphic { file_name, bytes, width, height })
}

fn add_file(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.addFile";
    let path = str_param(p, "path").ok_or_else(|| bad(CMD, "`path` is required"))?.to_string();
    let bytes = crate::file_cmds::read_file(&path)?;
    if bytes.len() as u64 > MAX_ITEM_BYTES {
        return Err(bad(CMD, format!("{path} is larger than {} MB", MAX_ITEM_BYTES >> 20)));
    }
    let doc = crate::file_cmds::import(&path, &bytes)?;
    let file_name = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).map(clip).filter(|n| !n.is_empty()).unwrap_or_else(|| "graphic".into());
    let stem = std::path::Path::new(&file_name).file_stem().and_then(|n| n.to_str()).unwrap_or("Graphic").to_string();
    add_item(s, p, CMD, &stem, Kind::Graphic { file_name, bytes: Arc::new(bytes), width: doc.size.width, height: doc.size.height })
}

fn add_pattern(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.addPattern";
    let pat = crate::pattern_cmds::resolve_param(s, CMD, p)?;
    let bytes = encode_pattern(&pat)?;
    let name = pat.display_name().to_string();
    add_item(s, p, CMD, &name, Kind::Pattern { bytes: Arc::new(bytes), width: pat.width, height: pat.height })
}

fn encode_pattern(pat: &photocraft_doc::Pattern) -> Result<Vec<u8>> {
    let size = photocraft_geom::Size::new(pat.width, pat.height);
    let f = pat.surface.format();
    let mut d = Document::new(format!("{}.pcraft", pat.display_name()), size, f.mode, f.sample);
    d.layers = vec![Layer::new(pat.display_name(), LayerContent::Raster(pat.surface.clone()))];
    crate::smart_cmds::encode_source(&d)
}

fn decode_pattern(name: &str, bytes: &[u8]) -> Result<photocraft_doc::Pattern> {
    let d = crate::smart_cmds::decode_source("pattern.pcraft", bytes)?;
    let surface = d
        .layers
        .iter()
        .find_map(|l| if let LayerContent::Raster(s) = &l.content { Some(s.clone()) } else { None })
        .ok_or_else(|| EngineError::Other("the pattern has no pixels".into()))?;
    Ok(photocraft_doc::Pattern::new(name, surface, d.size.width, d.size.height))
}

fn use_item(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.use";
    let (li, ii) = item_param(s, p, CMD)?;
    let item = s.libraries.list.get(li).and_then(|l| l.items.get(ii)).cloned().ok_or_else(|| bad(CMD, "no item"))?;
    match &item.kind {
        Kind::Color { rgb } => {
            let c = [rgb[0], rgb[1], rgb[2], 1.0];
            let target = str_param(p, "target").unwrap_or("foreground");
            match target {
                "foreground" => s.tools.foreground = c,
                "background" => s.tools.background = c,
                t => return Err(bad(CMD, format!("unknown target `{t}` (foreground|background)"))),
            }
            Ok(json!({"item": item.id, "target": target, "hex": hex(*rgb)}))
        }
        Kind::Graphic { file_name, bytes, .. } => {
            let linked = p.get("linked").and_then(Value::as_bool).unwrap_or(false);
            let path = if linked {
                let file = store_file_name(file_name, bytes);
                let path = s.preset_store.as_ref().and_then(|st| st.file_path(&file)).filter(|p| p.is_file());
                Some(path.and_then(|p| p.to_str().map(str::to_string)).ok_or_else(|| bad(CMD, "linked library items need the desktop app's library folder"))?)
            } else {
                None
            };
            let name = match path {
                Some(_) => file_name.clone(),
                None => item_place_name(&item.name, file_name),
            };
            let mut r = crate::file_cmds::place_bytes(s, &name, bytes.as_ref().clone(), path, p)?;
            r["item"] = json!(item.id);
            Ok(r)
        }
        Kind::Pattern { bytes, .. } => {
            let pat = decode_pattern(&item.name, bytes)?;
            let id = pat.id.clone();
            s.edit("Add Library Pattern", |doc, _| {
                if !doc.patterns.iter().any(|x| x.id == id) {
                    doc.patterns.push(pat);
                }
                Ok(())
            })?;
            Ok(json!({"item": item.id, "pattern": id}))
        }
    }
}

/// A preview of a graphic or pattern, fitting `side` px: `(width, height, RGBA8 unmultiplied)`.
/// `None` for colours and for bytes that no longer decode.
pub fn preview(item: &Item, side: u32) -> Option<(u32, u32, Vec<u8>)> {
    let (name, bytes) = match &item.kind {
        Kind::Color { .. } => return None,
        Kind::Graphic { file_name, bytes, .. } => (file_name.as_str(), bytes),
        Kind::Pattern { bytes, .. } => ("pattern.pcraft", bytes),
    };
    let img = crate::smart_cmds::source_image(name, bytes, photocraft_color::PixelFormat::RGBA8).ok()?;
    let b = img.bounds;
    let (w, h) = (b.width(), b.height());
    if w == 0 || h == 0 || side == 0 {
        return None;
    }
    let k = (side as f32 / w.max(h) as f32).min(1.0);
    let (tw, th) = (((w as f32 * k).round() as u32).clamp(1, side), ((h as f32 * k).round() as u32).clamp(1, side));
    let mut out = Vec::with_capacity((tw * th * 4) as usize);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    for y in 0..th {
        for x in 0..tw {
            let sx = b.x0.saturating_add(((x as f32 + 0.5) / k) as i32).min(b.x1 - 1);
            let sy = b.y0.saturating_add(((y as f32 + 0.5) / k) as i32).min(b.y1 - 1);
            let c = img.surface.rgba(sx, sy);
            out.extend_from_slice(&[q(c[0]), q(c[1]), q(c[2]), q(c[3])]);
        }
    }
    Some((tw, th, out))
}

/// The place name of a graphic: the item's name with the stored file's extension (the importer
/// picks the decoder by it).
fn item_place_name(name: &str, file_name: &str) -> String {
    match std::path::Path::new(file_name).extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{name}.{ext}"),
        None => file_name.to_string(),
    }
}

fn delete_item(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.deleteItem";
    let (li, ii) = item_param(s, p, CMD)?;
    let lib = s.libraries.list.get_mut(li).ok_or_else(|| bad(CMD, "no library"))?;
    if ii >= lib.items.len() {
        return Err(bad(CMD, "no item"));
    }
    let item = lib.items.remove(ii);
    s.libraries.changed();
    Ok(json!({"item": item.id, "name": item.name}))
}

fn rename_item(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.renameItem";
    let (li, ii) = item_param(s, p, CMD)?;
    let name = str_param(p, "name").map(clip).filter(|n| !n.is_empty()).ok_or_else(|| bad(CMD, "`name` is required"))?;
    let item = s.libraries.list.get_mut(li).and_then(|l| l.items.get_mut(ii)).ok_or_else(|| bad(CMD, "no item"))?;
    item.name = name.clone();
    let id = item.id;
    s.libraries.changed();
    Ok(json!({"item": id, "name": name}))
}

fn move_item(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "library.moveItem";
    let (li, ii) = item_param(s, p, CMD)?;
    let to = library_index(s, p, CMD, false)?;
    if to == li {
        return Ok(json!({"moved": false}));
    }
    let item = s.libraries.list.get_mut(li).map(|l| l.items.remove(ii)).ok_or_else(|| bad(CMD, "no item"))?;
    let lib = s.libraries.list.get_mut(to).ok_or_else(|| bad(CMD, "no library"))?;
    let mut item = item;
    item.name = unique_item_name(lib, &item.name);
    let id = item.id;
    lib.items.push(item);
    let library = lib.name.clone();
    s.libraries.changed();
    Ok(json!({"item": id, "library": library, "moved": true}))
}

pub fn specs() -> Vec<CommandSpec> {
    const LIB: &str = r#""library":name?=first (created as "My Library" when there is none)"#;
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let spec = |id: &'static str,
                label: &'static str,
                params: &'static str,
                enabled: fn(&Session) -> std::result::Result<(), String>,
                run: fn(&mut Session, &Value) -> Result<Value>| CommandSpec {
        id,
        label,
        menu: &[],
        shortcut: None,
        params,
        enabled,
        run,
        journal: !matches!(id, "library.list"),
    };
    vec![
        spec(
            "library.list",
            "Libraries",
            r#"{} → {libraries:[{name,items:[{id,name,kind:"color|graphic|pattern",hex?,rgb?,fileName?,width?,height?,bytes?}]}]}"#,
            always,
            list,
        ),
        spec("library.new", "New Library…", r#"{"name":str?="My Library"} → {library}"#, always, new_library),
        spec("library.delete", "Delete Library", r#"{"library":name} → {library,deleted}"#, always, delete_library),
        spec("library.rename", "Rename Library…", r#"{"library":name,"name":str} → {library}"#, always, rename_library),
        spec(
            "library.addColor",
            "Add Foreground Color",
            leak(format!(r##"{{"color":"#rrggbb"?=foreground,"name":str?,{LIB}}} → {{library,item}}"##)),
            always,
            add_color,
        ),
        spec(
            "library.addGraphic",
            "Add Graphic",
            leak(format!(r#"{{"layers":[id]?=selected,"name":str?,{LIB}}} → {{library,item}}. Packs the layers like Convert to Smart Object."#)),
            has_doc,
            add_graphic,
        ),
        spec("library.addFile", "Add Graphic from File…", leak(format!(r#"{{"path":str,"name":str?,{LIB}}} → {{library,item}}"#)), always, add_file),
        spec(
            "library.addPattern",
            "Add Pattern",
            leak(format!(
                r#"{{"pattern":id|name?=Patterns panel selection (document patterns first, then the library),"name":str?,{LIB}}} → {{library,item}}"#
            )),
            always,
            add_pattern,
        ),
        spec(
            "library.use",
            "Use Library Item",
            r#"{"item":id,"target":"foreground|background"="foreground" (colour),"linked":bool=false (graphic; desktop library folder),"scale":%?,"fit":bool=true,"center":[x,y]?} → colour {item,target,hex} | graphic {layer,scale,bounds,item} | pattern {item,pattern}"#,
            always,
            use_item,
        ),
        spec("library.deleteItem", "Delete Library Item", r#"{"item":id} → {item,name}"#, always, delete_item),
        spec("library.renameItem", "Rename Library Item…", r#"{"item":id,"name":str} → {item,name}"#, always, rename_item),
        spec("library.moveItem", "Move Library Item", r#"{"item":id,"library":name} → {item,library,moved}"#, always, move_item),
    ]
}

// ---------- persistence ----------

#[derive(Serialize, Deserialize)]
struct IndexFile {
    format: String,
    version: u32,
    libraries: Vec<StoredLibrary>,
}

#[derive(Serialize, Deserialize)]
struct StoredLibrary {
    name: String,
    items: Vec<StoredItem>,
}

#[derive(Serialize, Deserialize)]
struct StoredItem {
    name: String,
    kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rgb: Option<[f32; 3]>,
    /// The store file holding the bytes (graphics and patterns).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    /// A graphic's original file name (its extension picks the importer).
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "fileName")]
    file_name: Option<String>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

/// The extension a stored file keeps: the importer's, limited to store-safe characters.
fn ext_of(file_name: &str) -> String {
    std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.chars().filter(char::is_ascii_alphanumeric).take(8).collect::<String>().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .unwrap_or_else(|| "bin".into())
}

/// The store file of an item's bytes: `lib-<hash>.<ext>`, content-addressed.
pub fn store_file_name(file_name: &str, bytes: &[u8]) -> String {
    let h = blake3::hash(bytes).to_hex();
    format!("lib-{}.{}", &h[..32], ext_of(file_name))
}

fn stored_file(i: &Item) -> Option<String> {
    match &i.kind {
        Kind::Color { .. } => None,
        Kind::Graphic { file_name, bytes, .. } => Some(store_file_name(file_name, bytes)),
        Kind::Pattern { bytes, .. } => Some(store_file_name("pattern.pcraft", bytes)),
    }
}

/// Whether a store file name is one of ours.
pub fn is_library_file(name: &str) -> bool {
    !name.contains('/') && name.starts_with("lib-") && name.len() <= 64
}

/// Load the libraries (`None` when there is no index). Unreadable items are skipped with a
/// warning; their files stay on disk.
pub fn load(backend: &dyn PresetBackend, files: &[(String, u64)], warnings: &mut Vec<String>) -> Option<(Libraries, bool)> {
    if !files.iter().any(|(n, _)| n == INDEX) {
        return None;
    }
    let ix: IndexFile = match backend.read(INDEX, MAX_INDEX_BYTES).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())) {
        Ok(ix) => ix,
        Err(e) => {
            warnings.push(format!("libraries: {INDEX} skipped: {e}"));
            return Some((Libraries::default(), true));
        }
    };
    if ix.format != FORMAT || ix.version != 1 {
        warnings.push(format!("libraries: {INDEX} skipped: unsupported format {} v{}", ix.format, ix.version));
        return Some((Libraries::default(), true));
    }
    let mut libs = Libraries::default();
    let mut total = 0u64;
    let mut failed = false;
    for sl in ix.libraries.into_iter().take(MAX_LIBRARIES) {
        let mut lib = Library { name: clip(&sl.name), items: Vec::new() };
        if lib.name.is_empty() || libs.list.iter().any(|l| l.name.eq_ignore_ascii_case(&lib.name)) {
            lib.name = format!("{DEFAULT_LIBRARY} {}", libs.list.len() + 1);
        }
        for si in sl.items {
            if libs.items() + lib.items.len() >= MAX_ITEMS {
                break;
            }
            let name = clip(&si.name);
            let kind = match (si.kind.as_str(), si.rgb, si.file.as_deref()) {
                ("color", Some(rgb), _) => Some(Kind::Color { rgb: rgb.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }) }),
                ("graphic" | "pattern", _, Some(file)) if is_library_file(file) => {
                    let size = files.iter().find(|(n, _)| n == file).map(|(_, s)| *s);
                    match size {
                        Some(sz) if total.saturating_add(sz) <= MAX_TOTAL_BYTES => match backend.read(file, MAX_ITEM_BYTES) {
                            Ok(b) => {
                                total = total.saturating_add(b.len() as u64);
                                let bytes = Arc::new(b);
                                let (width, height) = (si.width.min(1 << 20), si.height.min(1 << 20));
                                Some(if si.kind == "pattern" {
                                    Kind::Pattern { bytes, width, height }
                                } else {
                                    let file_name =
                                        si.file_name.as_deref().map(clip).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("graphic.{}", ext_of(file)));
                                    Kind::Graphic { file_name, bytes, width, height }
                                })
                            }
                            Err(e) => {
                                warnings.push(format!("libraries: {file} skipped: {e}"));
                                None
                            }
                        },
                        Some(_) => {
                            warnings.push(format!("libraries: {file} skipped: the libraries are larger than {} MB", MAX_TOTAL_BYTES >> 20));
                            None
                        }
                        None => {
                            warnings.push(format!("libraries: item `{name}` skipped: {file} is missing"));
                            None
                        }
                    }
                }
                _ => {
                    warnings.push(format!("libraries: item `{name}` skipped: unreadable entry"));
                    None
                }
            };
            match kind {
                Some(kind) => {
                    let id = libs.fresh_id();
                    lib.items.push(Item { id, name, kind });
                }
                None => failed = true,
            }
        }
        libs.list.push(lib);
    }
    Some((libs, failed))
}

fn index_json(libs: &Libraries) -> std::result::Result<Vec<u8>, String> {
    let ix = IndexFile {
        format: FORMAT.into(),
        version: 1,
        libraries: libs
            .list
            .iter()
            .map(|l| StoredLibrary {
                name: l.name.clone(),
                items: l
                    .items
                    .iter()
                    .map(|i| {
                        let (rgb, file_name, width, height) = match &i.kind {
                            Kind::Color { rgb } => (Some(*rgb), None, 0, 0),
                            Kind::Graphic { file_name, width, height, .. } => (None, Some(file_name.clone()), *width, *height),
                            Kind::Pattern { width, height, .. } => (None, None, *width, *height),
                        };
                        StoredItem { name: i.name.clone(), kind: i.kind.key().into(), rgb, file: stored_file(i), file_name, width, height }
                    })
                    .collect(),
            })
            .collect(),
    };
    serde_json::to_vec_pretty(&ix).map_err(|e| e.to_string())
}

/// What the store last wrote for the libraries.
#[derive(Default)]
pub struct Synced {
    /// [`Libraries::rev`] written (or loaded).
    pub rev: Option<u64>,
    /// Item files on disk.
    pub files: std::collections::HashSet<String>,
    /// Keep files nothing references (a load failed: a fixed index may still need them).
    pub keep_orphans: bool,
    /// Whether [`INDEX`] exists (a session without libraries never creates it).
    pub has_index: bool,
}

/// Write the index and new item files, then delete item files nothing references.
pub fn sync(backend: &dyn PresetBackend, synced: &mut Synced, libs: &Libraries, warnings: &mut Vec<String>) {
    if synced.rev == Some(libs.rev) {
        return;
    }
    if libs.list.is_empty() && !synced.has_index && synced.files.is_empty() {
        synced.rev = Some(libs.rev);
        return;
    }
    let mut wanted = std::collections::HashSet::new();
    for i in libs.list.iter().flat_map(|l| l.items.iter()) {
        let (Some(file), Some(bytes)) = (stored_file(i), i.kind.bytes()) else { continue };
        if !synced.files.contains(&file)
            && let Err(e) = backend.write(&file, bytes)
        {
            warnings.push(format!("Couldn't save the library item `{}`: {e}", i.name));
            return;
        }
        synced.files.insert(file.clone());
        wanted.insert(file);
    }
    let bytes = match index_json(libs) {
        Ok(b) if b.len() as u64 <= MAX_INDEX_BYTES => b,
        Ok(_) => {
            warnings.push("Couldn't save the libraries: the list is too large".into());
            synced.rev = Some(libs.rev);
            return;
        }
        Err(e) => {
            warnings.push(format!("Couldn't save the libraries: {e}"));
            synced.rev = Some(libs.rev);
            return;
        }
    };
    if let Err(e) = backend.write(INDEX, &bytes) {
        warnings.push(format!("Couldn't save the libraries: {e}"));
        return;
    }
    synced.has_index = true;
    if !synced.keep_orphans {
        let orphans: Vec<String> = synced.files.iter().filter(|f| !wanted.contains(*f)).cloned().collect();
        for f in orphans {
            if backend.remove(&f).is_ok() {
                synced.files.remove(&f);
            }
        }
    }
    synced.rev = Some(libs.rev);
}

impl Session {
    /// Merge libraries loaded from the store into the session's (libraries made before the store
    /// finished loading are kept; a disk library with the same name gains their items).
    pub(crate) fn attach_libraries(&mut self, disk: Option<Libraries>) {
        let Some(disk) = disk else { return };
        if self.libraries.list.is_empty() {
            let rev = self.libraries.rev;
            self.libraries = disk;
            self.libraries.rev = rev;
            return;
        }
        for dl in disk.list {
            if self.libraries.list.len() >= MAX_LIBRARIES {
                break;
            }
            match self.libraries.list.iter_mut().find(|l| l.name.eq_ignore_ascii_case(&dl.name)) {
                Some(l) => {
                    for mut i in dl.items {
                        i.id = 0;
                        l.items.push(i);
                    }
                }
                None => self.libraries.list.push(Library {
                    name: dl.name,
                    items: dl
                        .items
                        .into_iter()
                        .map(|mut i| {
                            i.id = 0;
                            i
                        })
                        .collect(),
                }),
            }
        }
        // Renumber the merged items (ids from two sources may collide).
        let mut next = 0u64;
        for i in self.libraries.list.iter_mut().flat_map(|l| l.items.iter_mut()) {
            next += 1;
            i.id = next;
        }
        self.libraries.next_id = next;
        self.libraries.changed();
    }
}

#[cfg(test)]
mod tests;
