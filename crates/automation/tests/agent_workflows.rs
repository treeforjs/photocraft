//! More agent acceptance workflows over MCP (AUTO-219-10, FILE-216-6), on top of the ten in
//! `agent_tasks.rs`.
//!
//! Each workflow is its own test and uses only MCP tools (`doc_*`, `command_run`) the way an
//! agent would, checking the outcome through `doc_inspect`, `document.pixel` or exported files.

use photocraft_automation::{AuthorizedWorkspace, PhotocraftMcp};
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

/// A connected client, its server task and the test's workspace folder.
struct Agent {
    c: RunningService<RoleClient, Client>,
    server: tokio::task::JoinHandle<()>,
    dir: std::path::PathBuf,
}

impl Agent {
    async fn new(name: &str) -> Agent {
        let dir = std::env::temp_dir().join(format!("pc-agent-wf-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (s, c) = tokio::io::duplex(1 << 20);
        let workspace = AuthorizedWorkspace::new(Some(&dir), Some(&dir)).expect("test workspace");
        let server = tokio::spawn(async move {
            if let Ok(running) = PhotocraftMcp::headless_with_workspace(workspace).serve(s).await {
                let _ = running.waiting().await;
            }
        });
        Agent { c: Client.serve(c).await.expect("client init"), server, dir }
    }

    /// Closes the connection and waits for the server (and its open directory handles) to go
    /// before removing the folder; Windows refuses to delete a folder with open handles.
    async fn done(self) {
        self.c.cancel().await.unwrap();
        self.server.await.unwrap();
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    async fn tool(&self, name: &str, args: Value) -> Value {
        let mut p = CallToolRequestParams::new(name.to_owned());
        if let Value::Object(m) = args {
            p = p.with_arguments(m);
        }
        let r = self.c.call_tool(p).await.expect("call_tool transport");
        assert_ne!(r.is_error, Some(true), "{name} failed: {}", text(&r));
        serde_json::from_str(&text(&r)).unwrap_or(Value::String(text(&r)))
    }

    async fn run(&self, id: &str, params: Value) -> Value {
        self.tool("command_run", json!({"id": id, "params": params})).await
    }

    async fn new_doc(&self, w: u32, h: u32, background: &str) {
        self.tool("doc_new", json!({"width": w, "height": h, "background": background})).await;
    }

    async fn inspect(&self) -> Value {
        self.tool("doc_inspect", json!({})).await
    }

    /// Composite RGBA at (x, y).
    async fn pixel(&self, x: i32, y: i32) -> Vec<f64> {
        let v = self.run("document.pixel", json!({"x": x, "y": y})).await;
        let arr = if v.is_array() { v } else { v.get("result").cloned().unwrap_or(v) };
        arr.as_array().unwrap_or_else(|| panic!("pixel: {arr}")).iter().map(|x| x.as_f64().unwrap()).collect()
    }

    /// A new layer filled with `color` inside `rect` (x, y, w, h), deselected afterwards.
    async fn square(&self, name: &str, rect: [i32; 4], color: &str) {
        self.run("layer.new.layer", json!({"name": name})).await;
        self.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).await;
        self.run("edit.fill", json!({"color": color})).await;
        self.run("select.deselect", json!({})).await;
    }

    async fn layers(&self) -> Vec<Value> {
        self.inspect().await["layers"].as_array().cloned().unwrap_or_default()
    }
}

fn text(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n")
}

fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.len() >= b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

const WHITE: [f64; 4] = [1.0, 1.0, 1.0, 1.0];
const RED: [f64; 4] = [1.0, 0.0, 0.0, 1.0];

/// Levels stretches a dull grey to white (destructive), and undo brings it back.
#[tokio::test(flavor = "multi_thread")]
async fn levels_brighten_and_undo() {
    let a = Agent::new("levels").await;
    a.new_doc(32, 32, "#808080").await;
    let before = a.pixel(4, 4).await;
    a.run("image.adjustments.levels", json!({"inBlack": 0, "inWhite": 128})).await;
    assert!(close(&a.pixel(4, 4).await, &WHITE, 0.01), "levels to white");
    a.run("edit.undo", json!({})).await;
    assert!(close(&a.pixel(4, 4).await, &before, 1e-6));
    a.done().await;
}

/// A Hue/Saturation adjustment layer greys out red; at half opacity it is half way.
#[tokio::test(flavor = "multi_thread")]
async fn hue_saturation_layer_at_half_opacity() {
    let a = Agent::new("huesat").await;
    a.new_doc(16, 16, "#ff0000").await;
    a.run("layer.newAdjustmentLayer.hueSaturation", json!({"saturation": -100})).await;
    let grey = a.pixel(2, 2).await;
    assert!((grey[0] - grey[1]).abs() < 0.01 && (grey[1] - grey[2]).abs() < 0.01, "desaturated: {grey:?}");
    a.run("layer.setProps", json!({"opacity": 0.5})).await;
    let half = a.pixel(2, 2).await;
    assert!(half[0] > grey[0] + 0.1 && half[1] < 0.99 && (half[1] - grey[1] / 2.0).abs() < 0.03, "half way: {grey:?} -> {half:?}");
    assert_eq!(a.layers().await[0]["kind"], "Adjustment");
    a.done().await;
}

/// Grouping two layers and hiding the group shows the background.
#[tokio::test(flavor = "multi_thread")]
async fn group_and_hide() {
    let a = Agent::new("group").await;
    a.new_doc(20, 20, "white").await;
    a.square("A", [0, 0, 10, 20], "#ff0000").await;
    a.square("B", [10, 0, 10, 20], "#0000ff").await;
    a.run("select.allLayers", json!({})).await;
    a.run("layer.groupLayers", json!({"name": "Pair"})).await;
    let top = &a.layers().await[0];
    assert_eq!((top["kind"].as_str(), top["name"].as_str()), (Some("Group"), Some("Pair")), "{top}");
    assert!(close(&a.pixel(2, 2).await, &RED, 1e-4));
    a.run("layer.setProps", json!({"visible": false})).await;
    assert!(close(&a.pixel(2, 2).await, &WHITE, 1e-4));
    assert!(close(&a.pixel(15, 2).await, &WHITE, 1e-4));
    a.done().await;
}

/// Duplicate a layer, multiply it over itself, flatten: one layer with the multiplied colour.
#[tokio::test(flavor = "multi_thread")]
async fn duplicate_multiply_flatten() {
    let a = Agent::new("multiply").await;
    a.new_doc(16, 16, "white").await;
    a.square("Grey", [0, 0, 16, 16], "#808080").await;
    a.run("layer.duplicate", json!({})).await;
    a.run("layer.setProps", json!({"blend": "Multiply"})).await;
    let v = 128.0 / 255.0;
    let px = a.pixel(3, 3).await;
    assert!(close(&px, &[v * v, v * v, v * v, 1.0], 0.01), "{px:?}");
    a.run("layer.flattenImage", json!({})).await;
    assert_eq!(a.layers().await.len(), 1);
    assert!(close(&a.pixel(3, 3).await, &px, 1.0 / 255.0));
    a.done().await;
}

/// Canvas Size adds a black border around the centre.
#[tokio::test(flavor = "multi_thread")]
async fn canvas_size_adds_a_border() {
    let a = Agent::new("canvas").await;
    a.new_doc(40, 30, "white").await;
    a.run("image.canvasSize", json!({"width": 60, "height": 50, "anchor": "center", "extensionColor": "#000000"})).await;
    let d = a.inspect().await;
    assert_eq!((d["width"].as_u64(), d["height"].as_u64()), (Some(60), Some(50)));
    assert!(close(&a.pixel(1, 1).await, &[0.0, 0.0, 0.0, 1.0], 1e-4));
    assert!(close(&a.pixel(30, 25).await, &WHITE, 1e-4));
    a.done().await;
}

/// Trim crops away the transparent margin around the content.
#[tokio::test(flavor = "multi_thread")]
async fn trim_transparent_margins() {
    let a = Agent::new("trim").await;
    a.new_doc(64, 48, "transparent").await;
    a.square("Spot", [10, 8, 20, 12], "#00ff00").await;
    a.run("image.trim", json!({"basedOn": "transparent"})).await;
    let d = a.inspect().await;
    assert_eq!((d["width"].as_u64(), d["height"].as_u64()), (Some(20), Some(12)), "{d}");
    a.done().await;
}

/// A free transform scales a square layer to half size.
#[tokio::test(flavor = "multi_thread")]
async fn transform_scales_a_layer() {
    let a = Agent::new("transform").await;
    a.new_doc(64, 64, "white").await;
    a.square("Box", [10, 10, 20, 20], "#0000ff").await;
    a.run("edit.transform", json!({"rect": [10, 10, 30, 30], "quad": [[10, 10], [20, 10], [20, 20], [10, 20]], "interpolation": "nearest"})).await;
    let b = a.layers().await[0]["bounds"].clone();
    assert_eq!(b, json!([10, 10, 10, 10]), "{b}");
    assert!(close(&a.pixel(25, 25).await, &WHITE, 1e-4), "outside the scaled box");
    a.done().await;
}

/// Edit › Stroke draws a red outline inside a rectangular selection.
#[tokio::test(flavor = "multi_thread")]
async fn stroke_a_selection() {
    let a = Agent::new("stroke").await;
    a.new_doc(40, 40, "white").await;
    a.run("layer.new.layer", json!({"name": "Outline"})).await;
    a.run("select.rect", json!({"x": 10, "y": 10, "width": 20, "height": 20})).await;
    a.run("edit.stroke", json!({"width": 2, "color": "#ff0000", "location": "inside"})).await;
    a.run("select.deselect", json!({})).await;
    assert!(close(&a.pixel(10, 20).await, &RED, 1e-3), "the left edge is stroked");
    assert!(close(&a.pixel(20, 20).await, &WHITE, 1e-4), "the middle is not");
    assert!(close(&a.pixel(8, 20).await, &WHITE, 1e-4), "nothing outside");
    a.done().await;
}

/// Color Range at zero fuzziness selects exactly the red square.
#[tokio::test(flavor = "multi_thread")]
async fn color_range_finds_a_colour() {
    let a = Agent::new("colorrange").await;
    a.new_doc(32, 32, "white").await;
    a.square("Red", [8, 6, 16, 12], "#ff0000").await;
    a.run("select.colorRange", json!({"color": "#ff0000", "fuzziness": 0})).await;
    let d = a.inspect().await;
    assert_eq!(d["selectionBounds"], json!([8, 6, 16, 12]), "{}", d["selectionBounds"]);
    a.done().await;
}

/// A feathered fill has a soft edge: full inside, partial at the border, none far outside.
#[tokio::test(flavor = "multi_thread")]
async fn feathered_fill_has_a_soft_edge() {
    let a = Agent::new("feather").await;
    a.new_doc(64, 64, "white").await;
    a.run("select.rect", json!({"x": 16, "y": 16, "width": 32, "height": 32})).await;
    a.run("select.modify.feather", json!({"radius": 4})).await;
    a.run("edit.fill", json!({"color": "#000000"})).await;
    let (inside, edge, outside) = (a.pixel(32, 32).await, a.pixel(16, 32).await, a.pixel(4, 32).await);
    assert!(inside[0] < 0.02, "{inside:?}");
    assert!(edge[0] > 0.1 && edge[0] < 0.9, "soft edge {edge:?}");
    assert!(outside[0] > 0.98, "{outside:?}");
    a.done().await;
}

/// Inverse a selection and clear: only the centre of the layer survives.
#[tokio::test(flavor = "multi_thread")]
async fn inverse_and_clear_keeps_the_centre() {
    let a = Agent::new("inverse").await;
    a.new_doc(30, 30, "white").await;
    a.square("Red", [0, 0, 30, 30], "#ff0000").await;
    a.run("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).await;
    a.run("select.inverse", json!({})).await;
    a.run("edit.clear", json!({})).await;
    a.run("select.deselect", json!({})).await;
    assert!(close(&a.pixel(15, 15).await, &RED, 1e-4));
    assert!(close(&a.pixel(2, 2).await, &WHITE, 1e-4));
    assert_eq!(a.layers().await[0]["bounds"], json!([10, 10, 10, 10]));
    a.done().await;
}

/// Unsharp Mask raises the contrast at an edge between two greys.
#[tokio::test(flavor = "multi_thread")]
async fn unsharp_mask_sharpens_an_edge() {
    let a = Agent::new("sharpen").await;
    a.new_doc(40, 20, "#666666").await;
    a.square("Light", [20, 0, 20, 20], "#999999").await;
    a.run("layer.flattenImage", json!({})).await;
    let (dark, light) = (a.pixel(19, 10).await, a.pixel(20, 10).await);
    a.run("filter.sharpen.unsharpMask", json!({"amount": 200, "radius": 2, "threshold": 0})).await;
    let (dark2, light2) = (a.pixel(19, 10).await, a.pixel(20, 10).await);
    assert!(dark2[0] < dark[0] - 0.02 && light2[0] > light[0] + 0.02, "{dark:?}/{light:?} -> {dark2:?}/{light2:?}");
    assert!(close(&a.pixel(2, 10).await, &dark, 1.0 / 255.0), "far from the edge nothing changes");
    a.done().await;
}

/// Add Noise with a seed is repeatable; undo removes it.
#[tokio::test(flavor = "multi_thread")]
async fn seeded_noise_is_repeatable() {
    let a = Agent::new("noise").await;
    a.new_doc(16, 16, "#808080").await;
    let flat = a.pixel(5, 5).await;
    let params = json!({"amount": 40, "distribution": "gaussian", "monochromatic": true, "seed": 7});
    a.run("filter.noise.addNoise", params.clone()).await;
    let mut first = Vec::new();
    for i in 0..8 {
        first.push(a.pixel(i, i).await);
    }
    assert!(first.iter().any(|p| (p[0] - flat[0]).abs() > 0.02), "noise changed pixels");
    assert!(first.iter().all(|p| (p[0] - p[1]).abs() < 1e-6 && (p[1] - p[2]).abs() < 1e-6), "monochromatic");
    a.run("edit.undo", json!({})).await;
    assert!(close(&a.pixel(5, 5).await, &flat, 1e-6));
    a.run("filter.noise.addNoise", params).await;
    for (i, p) in first.iter().enumerate() {
        assert!(close(&a.pixel(i as i32, i as i32).await, p, 1e-6), "the same seed gives the same noise");
    }
    a.done().await;
}

/// Layer comps remember visibility: switch between two comps.
#[tokio::test(flavor = "multi_thread")]
async fn layer_comps_switch_visibility() {
    let a = Agent::new("comps").await;
    a.new_doc(16, 16, "white").await;
    a.square("Red", [0, 0, 16, 16], "#ff0000").await;
    a.run("layerComp.new", json!({"name": "With red"})).await;
    a.run("layer.setProps", json!({"visible": false})).await;
    a.run("layerComp.new", json!({"name": "Plain"})).await;
    assert!(close(&a.pixel(4, 4).await, &WHITE, 1e-4));
    a.run("layerComp.apply", json!({"comp": "With red"})).await;
    assert!(close(&a.pixel(4, 4).await, &RED, 1e-4));
    a.run("layerComp.apply", json!({"comp": "Plain"})).await;
    assert!(close(&a.pixel(4, 4).await, &WHITE, 1e-4));
    a.done().await;
}

/// A gradient fill layer runs dark to light from left to right.
#[tokio::test(flavor = "multi_thread")]
async fn gradient_fill_layer() {
    let a = Agent::new("gradient").await;
    a.new_doc(64, 16, "white").await;
    a.run("layer.newFillLayer.gradient", json!({"from": "#000000", "to": "#ffffff", "angle": 0, "style": "linear"})).await;
    let (l, m, r) = (a.pixel(1, 8).await, a.pixel(32, 8).await, a.pixel(62, 8).await);
    assert!(l[0] < m[0] && m[0] < r[0], "{l:?} {m:?} {r:?}");
    assert!(l[0] < 0.1 && r[0] > 0.9);
    assert_eq!(a.layers().await[0]["kind"], "Fill");
    a.done().await;
}

/// A green ellipse shape layer: filled at the centre, empty at the corner.
#[tokio::test(flavor = "multi_thread")]
async fn ellipse_shape_layer() {
    let a = Agent::new("shape").await;
    a.new_doc(48, 48, "white").await;
    a.run("shape.create", json!({"kind": "ellipse", "rect": [8, 8, 32, 32], "fill": "#00ff00"})).await;
    assert_eq!(a.layers().await[0]["kind"], "Shape");
    assert!(close(&a.pixel(24, 24).await, &[0.0, 1.0, 0.0, 1.0], 1e-3));
    assert!(close(&a.pixel(9, 9).await, &WHITE, 1e-3), "outside the ellipse's curve");
    a.done().await;
}

/// Rotating the canvas 90° clockwise swaps the size and moves the top-left mark to the top right.
#[tokio::test(flavor = "multi_thread")]
async fn rotate_canvas_clockwise() {
    let a = Agent::new("rotate").await;
    a.new_doc(40, 20, "white").await;
    a.square("Mark", [0, 0, 4, 4], "#ff0000").await;
    a.run("image.imageRotation.90cw", json!({})).await;
    let d = a.inspect().await;
    assert_eq!((d["width"].as_u64(), d["height"].as_u64()), (Some(20), Some(40)));
    assert!(close(&a.pixel(18, 1).await, &RED, 1e-4), "mark at the top right");
    assert!(close(&a.pixel(1, 1).await, &WHITE, 1e-4));
    a.done().await;
}

/// Grayscale conversion makes channels equal; converting back to RGB keeps the grey.
#[tokio::test(flavor = "multi_thread")]
async fn grayscale_and_back() {
    let a = Agent::new("gray").await;
    a.new_doc(8, 8, "#cc3366").await;
    a.run("image.mode.grayscale", json!({})).await;
    assert_eq!(a.inspect().await["mode"], "Grayscale");
    let g = a.pixel(2, 2).await;
    assert!((g[0] - g[1]).abs() < 1e-6 && (g[1] - g[2]).abs() < 1e-6, "{g:?}");
    a.run("image.mode.rgb", json!({})).await;
    assert_eq!(a.inspect().await["mode"], "Rgb");
    let back = a.pixel(2, 2).await;
    assert!((back[0] - back[2]).abs() < 0.01 && (back[0] - g[0]).abs() < 0.02, "{g:?} -> {back:?}");
    a.done().await;
}

/// A type layer rasterizes into pixels; undo gives the editable text back.
#[tokio::test(flavor = "multi_thread")]
async fn rasterize_type_and_undo() {
    let a = Agent::new("rasterize").await;
    a.new_doc(200, 80, "white").await;
    a.run("type.create", json!({"x": 10, "y": 60, "text": "Sale", "size": 40})).await;
    assert_eq!(a.layers().await[0]["kind"], "Type");
    a.run("type.rasterizeTypeLayer", json!({})).await;
    assert_eq!(a.layers().await[0]["kind"], "Pixel");
    a.run("edit.undo", json!({})).await;
    let t = &a.layers().await[0];
    assert_eq!((t["kind"].as_str(), t["text"]["text"].as_str()), (Some("Type"), Some("Sale")), "{t}");
    a.done().await;
}

/// A layer's own lock blocks painting on it, and unlocking allows it again.
#[tokio::test(flavor = "multi_thread")]
async fn locked_layer_refuses_a_fill() {
    let a = Agent::new("lock").await;
    a.new_doc(16, 16, "white").await;
    a.run("layer.new.layer", json!({"name": "Paint"})).await;
    a.run("layer.setProps", json!({"locks": {"pixels": true}})).await;
    let mut p = CallToolRequestParams::new("command_run");
    if let Value::Object(m) = json!({"id": "edit.fill", "params": {"color": "#ff0000"}}) {
        p = p.with_arguments(m);
    }
    let r = a.c.call_tool(p).await.expect("call_tool transport");
    assert_eq!(r.is_error, Some(true), "a locked layer is not filled: {}", text(&r));
    assert!(close(&a.pixel(4, 4).await, &WHITE, 1e-4));
    a.run("layer.setProps", json!({"locks": {"pixels": false}})).await;
    a.run("edit.fill", json!({"color": "#ff0000"})).await;
    assert!(close(&a.pixel(4, 4).await, &RED, 1e-4));
    a.done().await;
}
