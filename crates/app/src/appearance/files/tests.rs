use std::fs;

use super::*;

fn ok(_: &[u8]) -> Result<(), String> {
    Ok(())
}

#[test]
fn themes_are_scanned_sorted_with_caps_and_hidden_files_skipped() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("b.theme.json"), "{}").unwrap();
    fs::write(dir.path().join("a.theme.json"), "{\"x\":1}").unwrap();
    fs::write(dir.path().join(".hidden.theme.json"), "{}").unwrap();
    fs::write(dir.path().join("notes.json"), "{}").unwrap();
    let big = vec![b' '; usize::try_from(MAX_THEME_BYTES).unwrap() + 1];
    fs::write(dir.path().join("c.theme.json"), big).unwrap();
    let files = read_themes(dir.path());
    let names: Vec<_> = files.iter().map(|f| f.file_name.as_str()).collect();
    assert_eq!(names, ["a.theme.json", "b.theme.json", "c.theme.json"]);
    assert_eq!(files[0].contents.as_deref(), Ok("{\"x\":1}"));
    assert_eq!(files[2].contents, Err(FileProblem::TooLarge));
    assert!(read_themes(&dir.path().join("missing")).is_empty());
}

#[test]
fn only_the_first_64_themes_are_read() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..=MAX_THEME_FILES {
        fs::write(dir.path().join(format!("t{i:03}.theme.json")), "{}").unwrap();
    }
    let files = read_themes(dir.path());
    assert!(files[..MAX_THEME_FILES].iter().all(|f| f.contents.is_ok()));
    assert_eq!(files[MAX_THEME_FILES].contents, Err(FileProblem::TooMany));
}

#[test]
fn fonts_report_collections_and_web_formats() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["A.TTF", "b.otf", "c.ttc", "d.woff2", "e.txt"] {
        fs::write(dir.path().join(name), b"font").unwrap();
    }
    let files = read_fonts(dir.path());
    let by_name = |name: &str| {
        files
            .iter()
            .find(|f| f.file_name == name)
            .map(|f| f.bytes.clone())
    };
    assert_eq!(files.len(), 4);
    assert_eq!(by_name("A.TTF"), Some(Ok(b"font".to_vec())));
    assert_eq!(by_name("c.ttc"), Some(Err(FileProblem::Collection)));
    assert_eq!(by_name("d.woff2"), Some(Err(FileProblem::Unsupported)));
}

#[test]
fn a_rejected_copy_never_touches_the_existing_file() {
    let source_dir = tempfile::tempdir().unwrap();
    let fonts = tempfile::tempdir().unwrap();
    fs::write(fonts.path().join("Mono.ttf"), b"good").unwrap();
    let source = source_dir.path().join("Mono.ttf");
    fs::write(&source, b"bad").unwrap();
    let rejected = copy_font(&source, fonts.path(), |_| Err("not a font".to_owned()));
    assert_eq!(rejected, Err(CopyProblem::Invalid("not a font".to_owned())));
    assert_eq!(fs::read(fonts.path().join("Mono.ttf")).unwrap(), b"good");

    fs::write(&source, b"better").unwrap();
    assert_eq!(
        copy_font(&source, fonts.path(), ok),
        Ok("Mono.ttf".to_owned())
    );
    assert_eq!(fs::read(fonts.path().join("Mono.ttf")).unwrap(), b"better");
    let leftovers = fs::read_dir(fonts.path()).unwrap().count();
    assert_eq!(leftovers, 1, "no temp file is left behind");

    let collection = source_dir.path().join("Set.ttc");
    fs::write(&collection, b"ttcf").unwrap();
    assert_eq!(
        copy_font(&collection, fonts.path(), ok),
        Err(CopyProblem::File(FileProblem::Collection))
    );
}

#[test]
fn a_full_folder_only_accepts_replacements() {
    let source_dir = tempfile::tempdir().unwrap();
    let fonts = tempfile::tempdir().unwrap();
    for i in 0..MAX_FONT_FILES {
        fs::write(fonts.path().join(format!("f{i:03}.ttf")), b"x").unwrap();
    }
    let new = source_dir.path().join("new.ttf");
    fs::write(&new, b"y").unwrap();
    assert_eq!(
        copy_font(&new, fonts.path(), ok),
        Err(CopyProblem::File(FileProblem::TooMany))
    );
    let replacement = source_dir.path().join("f000.ttf");
    fs::write(&replacement, b"z").unwrap();
    assert_eq!(
        copy_font(&replacement, fonts.path(), ok),
        Ok("f000.ttf".to_owned())
    );
}

#[test]
fn the_signature_moves_on_add_and_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let empty = signature(dir.path());
    fs::write(dir.path().join("a.theme.json"), "{}").unwrap();
    let one = signature(dir.path());
    assert_ne!(empty, one);
    assert_eq!(one, signature(dir.path()));
    fs::write(dir.path().join("a.theme.json"), "{ }").unwrap();
    assert_ne!(one, signature(dir.path()));
}
