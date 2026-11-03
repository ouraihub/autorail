use super::*;

#[test]
fn code_and_diff_lines_wrap_headers_do_not() {
    let rows = vec![
        Row::FileHeader("a-long-path/that/does/not/wrap.rs".into()),
        Row::Hunk("@@ -1,2 +1,2 @@ a long hunk header".into()),
        Row::Diff(diff::Line {
            kind: diff::Kind::Added,
            old: None,
            new: Some(1),
            text: "0123456789".into(),
        }),
        Row::Code {
            number: 7,
            text: String::new(),
        },
    ];
    let pieces = cut(&rows, 4);
    let rows_of: Vec<usize> = pieces.iter().map(|p| p.row).collect();
    assert_eq!(rows_of, vec![0, 1, 2, 2, 2, 3]);
    assert_eq!(pieces[2].range, 0..4, "the first piece starts the line");
    assert_eq!(pieces[4].range, 8..10);
    assert_eq!(pieces[5].range, 0..0, "an empty line still draws");
}
