//! P7-M1 Office pack slice 1 (ADR 0055), interactive: needs Microsoft Office desktop. Works on
//! temp files only; calls the skill functions directly (the policy prompt for file changes is
//! the existing, separately tested path). Checks that no Office process XtraOS started is left.
use serde_json::json;
use xtraos_core::office;

fn office_pids() -> Vec<u32> {
    let out = std::process::Command::new("tasklist")
        .args(["/FO", "CSV", "/NH"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            let l = l.to_lowercase();
            l.starts_with("\"excel.exe\"") || l.starts_with("\"winword.exe\"") || l.starts_with("\"powerpnt.exe\"")
        })
        .filter_map(|l| l.split("\",\"").nth(1).and_then(|p| p.parse().ok()))
        .collect()
}

#[test]
#[ignore = "interactive: needs Microsoft Office; uses temp files only"]
fn p7m1_word_append_and_read() {
    let before = office_pids();
    let dir = std::env::temp_dir().join(format!("xtraos-word-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let doc = dir.join("minutes.docx");

    office::word_append(&json!({"path": doc.display().to_string(), "text": "Meeting minutes\nAttendees: Om, Asha"})).unwrap();
    assert!(doc.exists());
    office::word_append(&json!({"path": doc.display().to_string(), "text": "Decision: ship on Friday."})).unwrap();
    let r = office::word_read(&json!({"path": doc.display().to_string()})).unwrap();
    let text = r["text"].as_str().unwrap();
    eprintln!("{text}");
    assert_eq!(r["paragraphs"], 3, "{r}");
    assert!(text.contains("Meeting minutes\nAttendees: Om, Asha\nDecision: ship on Friday."), "{text}");
    assert!(office::word_append(&json!({"path": doc.display().to_string(), "text": "x", "save_as": doc.display().to_string()})).is_err(), "never overwrites");

    std::thread::sleep(std::time::Duration::from_secs(12));
    let left: Vec<u32> = office_pids().into_iter().filter(|p| !before.contains(p)).collect();
    assert!(left.is_empty(), "Office processes left behind: {left:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "interactive: needs Microsoft Office; uses temp files only"]
fn p7m1_excel_write_read_and_pdf() {
    let before = office_pids();
    let dir = std::env::temp_dir().join(format!("xtraos-office-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let book = dir.join("sales.xlsx");

    // A new workbook from values.
    let t0 = std::time::Instant::now();
    let w = office::excel_write(&json!({
        "path": book.display().to_string(), "range": "A1",
        "values": [["Item", "Qty", "Price"], ["Pens", 12, 1.5], ["Paper", 3, 4.25]]
    }))
    .unwrap();
    eprintln!("write: {} ({} ms)", w["text"], t0.elapsed().as_millis());
    assert!(book.exists());

    // Read it back: numbers stay numbers.
    let r = office::excel_read(&json!({"path": book.display().to_string(), "range": "A1:C3"})).unwrap();
    eprintln!("{}", r["text"].as_str().unwrap());
    assert_eq!(r["values"][0], json!(["Item", "Qty", "Price"]));
    assert_eq!(r["values"][1], json!(["Pens", 12, 1.5]));
    assert_eq!(r["values"][2][2], json!(4.25));

    // Update a cell in place, then read it.
    office::excel_write(&json!({"path": book.display().to_string(), "range": "B3", "values": [[30]]})).unwrap();
    let r = office::excel_read(&json!({"path": book.display().to_string(), "range": "B3"})).unwrap();
    assert_eq!(r["values"], json!([[30]]));

    // PDF, never overwriting.
    let p = office::export_pdf(&json!({"path": book.display().to_string()})).unwrap();
    assert!(dir.join("sales.pdf").exists(), "{p}");
    assert!(office::export_pdf(&json!({"path": book.display().to_string()})).is_err(), "never overwrites");

    // Word too (a .docx made by Word itself via a write is out of scope: use an RTF file).
    let rtf = dir.join("note.rtf");
    std::fs::write(&rtf, r"{\rtf1\ansi Hello from XtraOS\par}").unwrap();
    office::export_pdf(&json!({"path": rtf.display().to_string()})).unwrap();
    assert!(dir.join("note.pdf").exists());

    // Errors: a range too big, a missing sheet.
    assert!(office::excel_read(&json!({"path": book.display().to_string(), "range": "A1:Z200"})).is_err());
    assert!(office::excel_read(&json!({"path": book.display().to_string(), "range": "A1", "sheet": "Nope"})).is_err());

    // No Office process left that this test started (each one gets ≤ 10 s to exit in the
    // background, then it is ended).
    std::thread::sleep(std::time::Duration::from_secs(12));
    let left: Vec<u32> = office_pids().into_iter().filter(|p| !before.contains(p)).collect();
    assert!(left.is_empty(), "Office processes left behind: {left:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[ignore = "interactive: needs Microsoft Office; uses temp files only"]
fn p7m1_powerpoint_add_read_export() {
    let before = office_pids();
    let dir = std::env::temp_dir().join(format!("xtraos-ppt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let deck = dir.join("plan.pptx");
    let p = deck.display().to_string();

    // A new deck, a second slide, then one inserted first (`at`), a title-only slide.
    office::ppt_add_slide(&json!({"path": p, "title": "Q4 plan", "bullets": ["Ship v2", "Hire two engineers"]})).unwrap();
    assert!(deck.exists());
    office::ppt_add_slide(&json!({"path": p, "title": "Risks", "bullets": ["Supplier delay"]})).unwrap();
    let r = office::ppt_add_slide(&json!({"path": p, "title": "Agenda", "at": 1})).unwrap();
    assert_eq!(r["slide"], 1, "{r}");

    let r = office::ppt_read(&json!({"path": p, "notes": true})).unwrap();
    let text = r["text"].as_str().unwrap();
    eprintln!("{text}");
    let slides = r["slides"].as_array().unwrap();
    assert_eq!(slides.len(), 3, "{r}");
    assert_eq!(slides[0]["title"], "Agenda");
    assert_eq!(slides[1]["title"], "Q4 plan");
    assert_eq!(slides[1]["text"][0], "Ship v2\nHire two engineers", "{r}");
    assert_eq!(slides[2]["title"], "Risks");
    assert!(text.contains("[2] Q4 plan\n  Ship v2\n  Hire two engineers"), "{text}");

    // save_as writes a copy and never overwrites.
    let copy = dir.join("plan-copy.pptx");
    office::ppt_add_slide(&json!({"path": p, "title": "Extra", "save_as": copy.display().to_string()})).unwrap();
    assert_eq!(office::ppt_read(&json!({"path": copy.display().to_string()})).unwrap()["slides"].as_array().unwrap().len(), 4);
    assert_eq!(office::ppt_read(&json!({"path": p})).unwrap()["slides"].as_array().unwrap().len(), 3, "the original is unchanged");
    assert!(office::ppt_add_slide(&json!({"path": p, "title": "x", "save_as": copy.display().to_string()})).is_err());
    assert!(office::ppt_add_slide(&json!({"path": dir.join("new.ppt").display().to_string(), "title": "x"})).is_err(), "new decks are .pptx");

    office::export_pdf(&json!({"path": p})).unwrap();
    assert!(dir.join("plan.pdf").exists());

    std::thread::sleep(std::time::Duration::from_secs(12));
    let left: Vec<u32> = office_pids().into_iter().filter(|p| !before.contains(p)).collect();
    assert!(left.is_empty(), "Office processes left behind: {left:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
