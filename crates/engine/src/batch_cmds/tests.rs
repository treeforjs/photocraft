use super::*;
use crate::jobs::{JobOutcome, Started};

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-batch-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// Writes a solid-colour PNG and returns its path.
fn png(dir: &str, name: &str, color: &str) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6, "background": color})).unwrap();
    let path = join(dir, name);
    save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    path
}

fn names(r: &Value, root: &str) -> Vec<String> {
    r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap().strip_prefix(root).unwrap().trim_start_matches('/').to_string()).collect()
}

fn naming(template: &str, start: u64) -> Naming {
    Naming::from_params(&json!({"naming": template, "startSerial": start, "date": "2026-03-09"})).unwrap()
}

#[test]
fn naming_templates_render_every_token() {
    assert_eq!(naming("{name}", 1).render("photo", "jpg", 0).unwrap(), "photo.jpg");
    assert_eq!(naming("{name}_{serial:3}", 7).render("a", "png", 0).unwrap(), "a_007.png");
    assert_eq!(naming("{NAME}-{lname}-{serial}", 1).render("MixEd", "png", 4).unwrap(), "MIXED-mixed-5.png");
    assert_eq!(naming("{letter}{LETTER}", 1).render("x", "png", 0).unwrap(), "aA.png");
    assert_eq!(naming("{letter}", 26).render("x", "png", 0).unwrap(), "z.png");
    assert_eq!(naming("{letter}", 27).render("x", "png", 0).unwrap(), "aa.png");
    assert_eq!(naming("{letter}", 1).render("x", "png", 701).unwrap(), "zz.png");
    assert_eq!(naming("{yyyymmdd} {yymmdd} {mmddyy} {mmdd} {ddmmyy} {ddmm}", 1).render("x", "tif", 0).unwrap(), "20260309 260309 030926 0309 090326 0903.tif");
    assert_eq!(naming("{name}.{EXT}.{ext}", 1).render("x", "jpg", 0).unwrap(), "x.JPG.jpg.jpg");
    assert_eq!(naming("{{{name}}}", 1).render("x", "png", 0).unwrap(), "{x}.png");
    // A name can't leave the destination folder.
    assert_eq!(naming("../{name}", 1).render("x", "png", 0).unwrap(), ".._x.png");
    assert!(naming("..", 1).render("x", "png", 0).is_err());
    for bad in ["", "{nope}", "{name", "name}", "{serial:0}", "{serial:x}", "{serial:10}"] {
        assert!(Naming::from_params(&json!({"naming": bad})).is_err(), "{bad:?}");
    }
    assert!(Naming::from_params(&json!({"date": "March 9"})).is_err());
    assert!(Naming::from_params(&json!({"startSerial": -1})).is_err());
    // The default date is today's.
    let today = Naming::from_params(&json!({})).unwrap().date;
    assert!(today.0 >= 2026, "{today:?}");
}

#[test]
fn subfolders_are_included_on_request_and_mirrored_in_the_destination() {
    let dir = tmp("tree");
    let input = join(&dir, "in");
    png(&input, "a.png", "#ff0000");
    png(&join(&input, "sub"), "b.png", "#00ff00");
    png(&join(&input, "sub/deep"), "c.png", "#0000ff");
    png(&join(&input, ".hidden"), "d.png", "#ffffff");
    let mut s = Session::new();
    let out = join(&dir, "flat");
    let r = s.execute(CMD, json!({"steps": [], "input": input, "output": out})).unwrap();
    assert_eq!(names(&r, &out), ["a.png"]);
    let out = join(&dir, "deep");
    let r = s
        .execute(CMD, json!({"steps": [["image.adjustments.invert", {}]], "input": input, "output": out, "includeSubfolders": true, "format": "jpg"}))
        .unwrap();
    assert_eq!(names(&r, &out), ["a.jpg", "sub/b.jpg", "sub/deep/c.jpg"], "{r}");
    assert!(std::path::Path::new(&join(&out, "sub/deep/c.jpg")).is_file());
    // The same name in two subfolders doesn't collide.
    png(&join(&input, "sub"), "a.png", "#000000");
    let out = join(&dir, "again");
    let r = s.execute(CMD, json!({"steps": [], "input": input, "output": out, "includeSubfolders": true})).unwrap();
    assert_eq!(names(&r, &out), ["a.png", "sub/a.png", "sub/b.png", "sub/deep/c.png"], "{r}");
}

#[cfg(unix)]
#[test]
fn a_symlinked_folder_cycle_is_not_followed() {
    let dir = tmp("cycle");
    let input = join(&dir, "in");
    png(&input, "a.png", "#ff0000");
    std::os::unix::fs::symlink(&input, join(&input, "loop")).unwrap();
    let mut s = Session::new();
    let out = join(&dir, "out");
    let r = s.execute(CMD, json!({"steps": [], "input": input, "output": out, "includeSubfolders": true})).unwrap();
    assert_eq!(names(&r, &out), ["a.png"]);
}

#[test]
fn open_documents_are_processed_as_copies() {
    let dir = tmp("open");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 4, "background": "#ff0000", "name": "Red"})).unwrap();
    s.execute("file.new", json!({"width": 4, "height": 4, "background": "#0000ff", "name": "Blue"})).unwrap();
    let before: Vec<_> = s.documents().iter().map(|d| (d.doc.clone(), d.history.past_len())).collect();
    let out = join(&dir, "out");
    let r = s
        .execute(CMD, json!({"steps": [["image.adjustments.invert", {}]], "input": "open", "output": out, "format": "png", "naming": "{serial:2}-{name}"}))
        .unwrap();
    assert_eq!(names(&r, &out), ["01-Red.png", "02-Blue.png"], "{r}");
    for ((doc, steps), d) in before.iter().zip(s.documents()) {
        assert!(Arc::ptr_eq(doc, &d.doc), "the open document is unchanged");
        assert_eq!(d.history.past_len(), *steps);
    }
    let back = photocraft_io::import("x.png", &std::fs::read(join(&out, "01-Red.png")).unwrap()).unwrap().document;
    let px = photocraft_compose::flatten(&back).px[0];
    assert!(px[0] < 0.01 && px[1] > 0.99 && px[2] > 0.99, "red inverted to cyan: {px:?}");
    assert!(Session::new().execute(CMD, json!({"steps": [], "input": "open", "output": out})).is_err(), "no open documents");
}

#[test]
fn errors_are_logged_and_can_stop_the_batch() {
    let dir = tmp("errors");
    let input = join(&dir, "in");
    let a = png(&input, "a.png", "#ff0000");
    let missing = join(&input, "missing.png");
    let mut s = Session::new();
    let out = join(&dir, "out");
    let log = join(&dir, "log.txt");
    let r = s.execute(CMD, json!({"steps": [], "input": [missing, a], "output": out, "errorLog": log})).unwrap();
    assert_eq!((r["files"].as_array().unwrap().len(), r["errors"].as_array().unwrap().len()), (1, 1), "{r}");
    assert_eq!(r["errorLog"], json!(log));
    assert!(r.get("stopped").is_none());
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.starts_with(&format!("{missing}: ")) && text.lines().count() == 1, "{text}");
    let out = join(&dir, "stop");
    let r = s.execute(CMD, json!({"steps": [], "input": [missing, a], "output": out, "stopOnError": true})).unwrap();
    assert_eq!((r["files"].as_array().unwrap().len(), r["errors"].as_array().unwrap().len()), (0, 1), "{r}");
    assert_eq!(r["stopped"], json!(true));
    // A failing step names itself.
    let r = s.execute(CMD, json!({"steps": [["layer.delete", {"layer": 999_999}]], "input": [a], "output": out})).unwrap();
    let e = r["errors"][0]["error"].as_str().unwrap();
    assert!(e.starts_with("step 1 (layer.delete)"), "{e}");
}

fn wait_for_end(s: &mut Session) -> Vec<crate::jobs::JobEvent> {
    let t = std::time::Instant::now();
    loop {
        let ev = s.poll_jobs();
        if !ev.is_empty() {
            return ev;
        }
        assert!(t.elapsed() < std::time::Duration::from_secs(60), "the batch never ended");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn a_background_batch_reports_progress_and_its_result() {
    let dir = tmp("job");
    let input = join(&dir, "in");
    for i in 0..3 {
        png(&input, &format!("{i}.png"), "#808080");
    }
    let mut s = Session::new();
    let out = join(&dir, "out");
    let Started::Job(id) = s.start(CMD, json!({"steps": [["image.adjustments.invert", {}]], "input": input, "output": out})).unwrap() else {
        panic!("ran inline")
    };
    let ev = wait_for_end(&mut s);
    let JobOutcome::Done(r) = &ev[0].outcome else { panic!("{ev:?}") };
    assert_eq!(ev[0].id, id);
    assert_eq!(names(r, &out), ["0.png", "1.png", "2.png"]);
}

#[test]
fn cancelling_a_background_batch_stops_before_the_next_file() {
    let dir = tmp("cancel");
    let input = join(&dir, "in");
    const N: usize = 200;
    for i in 0..N {
        png(&input, &format!("{i:03}.png"), "#808080");
    }
    let mut s = Session::new();
    let out = join(&dir, "out");
    let Started::Job(id) = s.start(CMD, json!({"steps": [["filter.blur.gaussianBlur", {"radius": 3}]], "input": input, "output": out})).unwrap() else {
        panic!("ran inline")
    };
    assert!(s.cancel_job(id));
    let ev = s.poll_jobs();
    assert!(ev.iter().any(|e| e.id == id && e.outcome == JobOutcome::Cancelled), "{ev:?}");
    s.join_cancelled_jobs();
    let written = std::fs::read_dir(&out).map(|d| d.count()).unwrap_or(0);
    assert!(written < N, "the batch stopped early ({written} of {N} written)");
}
