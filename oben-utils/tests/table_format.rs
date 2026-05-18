use std::io::Write;

/// Helper that captures stdout by redirecting to a Vec<u8>.
fn capture_output<F: FnOnce(&mut dyn Write)>(f: F) -> String {
    let mut buf = Vec::<u8>::new();
    f(&mut buf);
    String::from_utf8(buf).unwrap()
}

/// Compute column widths — mirrors the logic in print_table.
fn compute_widths(headers: &[&str], rows: &[Vec<String>]) -> Vec<usize> {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }
    widths
}

#[test]
fn compute_widths_single_row() {
    let headers = &["ID", "Max Tokens", "Owned By"];
    let rows = vec![vec!["qwen35-local".to_string(), "262144".to_string(), "vllm".to_string()]];
    let widths = compute_widths(headers, &rows);
    assert_eq!(widths, vec![12, 10, 8]); // "qwen35-local", "Max Tokens", "Owned By"
}

#[test]
fn compute_widths_multiple_rows() {
    let headers = &["A", "B"];
    let rows = vec![
        vec!["1".to_string(), "xy".to_string()],
        vec!["123".to_string(), "x".to_string()],
    ];
    let widths = compute_widths(headers, &rows);
    assert_eq!(widths, vec![3, 2]);
}

#[test]
fn compute_widths_empty_rows() {
    let headers = &["A", "B"];
    let rows: Vec<Vec<String>> = vec![];
    let widths = compute_widths(headers, &rows);
    assert_eq!(widths, vec![1, 1]);
}

#[test]
fn print_table_single_row() {
    let output = capture_output(|w| {
        let headers = &["ID", "Max Tokens", "Owned By"];
        let rows = vec![
            vec!["qwen35-local".to_string(), "262144".to_string(), "vllm".to_string()],
        ];
        oben_utils::terminal::print_table(headers, rows, w);
    });

    assert!(output.contains("ID"));
    assert!(output.contains("Max Tokens"));
    assert!(output.contains("Owned By"));
    assert!(output.contains("qwen35-local"));
    assert!(output.contains("262144"));
    assert!(output.contains("vllm"));
}

#[test]
fn print_table_multiple_rows() {
    let output = capture_output(|w| {
        let headers = &["A", "B"];
        let rows = vec![
            vec!["1".to_string(), "x".to_string()],
            vec!["2".to_string(), "y".to_string()],
        ];
        oben_utils::terminal::print_table(headers, rows, w);
    });

    assert!(output.contains("A"));
    assert!(output.contains("B"));
    assert!(output.contains("1"));
    assert!(output.contains("2"));
    assert!(output.contains("x"));
    assert!(output.contains("y"));
    // Should have 2 data rows + header + separator
    let line_count = output.lines().count();
    assert_eq!(line_count, 4); // header + separator + 2 rows
}

#[test]
fn print_table_empty_rows() {
    let output = capture_output(|w| {
        let headers = &["ID", "Value"];
        let rows: Vec<Vec<String>> = vec![];
        oben_utils::terminal::print_table(headers, rows, w);
    });

    // print_table returns early when rows are empty — no output
    assert!(output.is_empty());
}
