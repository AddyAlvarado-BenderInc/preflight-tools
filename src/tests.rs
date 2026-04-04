use super::*;
use crate::filter::{
    block_is_outside_image, operands_to_matrix, operands_to_rect, re_is_outside,
    remove_outside_re_f_pairs,
};

/*
    To run these tests, use:
        cargo test -- --nocapture
    The --nocapture flag allows us to see printed output during tests.

    To specifically run just one test, use:
        cargo test test_name -- --nocapture
*/

// Shared helper -- loads the source PDF once and returns the document.
// Paths are relative to the project root via CARGO_MANIFEST_DIR.
const SOURCE_PDF_REL: &str = "test/test_assets/pdf_test_data_print_v2.pdf";
const GOAL_PDF_REL: &str = "test/test_assets/pdf_test_data_print_v2_final_goal.pdf";

fn test_asset(rel: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn load_document(path: &std::path::Path) -> lopdf::Document {
    let file = std::fs::File::open(path).expect("test PDF not found");
    lopdf::Document::load_from(file).expect("failed to parse PDF")
}

// -- Step 1 & 2 checkpoints --------------------------------------------

#[test]
fn pdf_loads_and_has_one_page() {
    let doc = load_document(&test_asset(SOURCE_PDF_REL));
    assert_eq!(doc.get_pages().len(), 1, "expected single-page PDF");
    println!("PDF loaded with 1 page as expected.");
    println!("\n");
}

#[test]
fn content_stream_parses_to_operations() {
    let doc = load_document(&test_asset(SOURCE_PDF_REL));
    let page_id = doc.get_pages()[&1];
    let content = doc.get_and_decode_page_content(page_id).unwrap();
    // We know from analysis the source has ~728 operations.
    // Exact count may vary slightly by lopdf version, so just assert non-empty
    // and above a floor we know is true.
    assert!(
        content.operations.len() > 50,
        "expected many operations, got {}",
        content.operations.len()
    );
    println!(
        "Parsed {} operations from content stream.",
        content.operations.len()
    );
    println!("\n");
}

#[test]
fn content_stream_contains_expected_operators() {
    let doc = load_document(&test_asset(SOURCE_PDF_REL));
    let page_id = doc.get_pages()[&1];
    let content = doc.get_and_decode_page_content(page_id).unwrap();
    let ops: Vec<&str> = content
        .operations
        .iter()
        .map(|o| o.operator.as_str())
        .collect();

    // Operators we know must appear in this PDF
    for expected in &["q", "Q", "cm", "re", "f", "Do", "BT", "ET", "W", "n"] {
        assert!(
            ops.contains(expected),
            "expected operator {:?} not found in content stream",
            expected
        );
    }
    println!(
        "All expected operators found in content stream. Here's a sample: {:?}",
        &ops[0..10]
    );
    println!("\n");
}

// -- TrimBox checkpoint ------------------------------------------------

#[test]
fn trim_box_has_correct_values() {
    let doc = load_document(&test_asset(SOURCE_PDF_REL));
    let page_id = doc.get_pages()[&1];
    let page_dict = doc.get_dictionary(page_id).unwrap();

    let trim = page_dict
        .get(b"TrimBox")
        .expect("TrimBox not found")
        .as_array()
        .expect("TrimBox is not an array");

    // From our analysis: TrimBox = [30, 30, 642, 822]
    let values: Vec<f64> = trim
        .iter()
        .map(|v| match v {
            lopdf::Object::Integer(i) => *i as f64,
            lopdf::Object::Real(r) => *r as f64,
            _ => panic!("TrimBox value is not a number: {:?}", v),
        })
        .collect();

    assert_eq!(values.len(), 4, "TrimBox should have 4 values");
    assert!((values[0] - 30.0).abs() < 0.01, "TrimBox x0 should be 30");
    assert!((values[1] - 30.0).abs() < 0.01, "TrimBox y0 should be 30");
    assert!((values[2] - 642.0).abs() < 0.01, "TrimBox x1 should be 642");
    assert!((values[3] - 822.0).abs() < 0.01, "TrimBox y1 should be 822");
    println!("TrimBox values: {:?}", values);
    println!("\n");
}

// -- Matrix math unit tests --------------------------------------------

#[test]
fn matrix_identity_leaves_point_unchanged() {
    let m = Matrix::identity();
    let (x, y) = m.transform_point(100.0, 200.0);
    assert!((x - 100.0).abs() < 1e-10);
    assert!((y - 200.0).abs() < 1e-10);
    println!("Identity matrix leaves point unchanged: ({}, {})", x, y);
    println!("\n");
}

#[test]
fn matrix_translation_moves_point() {
    // a=1 b=0 c=0 d=1 e=50 f=75 -> pure translation by (50, 75)
    let m = Matrix::from_values(1.0, 0.0, 0.0, 1.0, 50.0, 75.0);
    let (x, y) = m.transform_point(10.0, 20.0);
    assert!((x - 60.0).abs() < 1e-10);
    assert!((y - 95.0).abs() < 1e-10);
    println!("Translation matrix moves point to: ({}, {})", x, y);
    println!("\n");
}

#[test]
fn matrix_known_ctm_places_red_rect_outside_trim() {
    // This is the actual CTM from the source PDF's PlacedPDF block.
    // The red rectangle at local coords (298.292, -312.455, 7.879, 60.394)
    // should land outside TrimBox right edge (642) after transformation.
    let ctm = Matrix::from_values(1.02883, 0.0, 0.0, -1.03942, 336.0, 426.0);
    let red_rect = Rect::new(298.292, -312.455, 7.879, 60.394);
    let in_page_space = ctm.transform_rect(&red_rect);
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);

    assert!(
        in_page_space.is_outside(&trim),
        "red rect left edge {:.4} should be outside trim right 642",
        in_page_space.x
    );
    println!("Red rect transformed left edge: {:.4}", in_page_space.x);
    println!("\n");
}

#[test]
fn matrix_known_ctm_keeps_blue_rect_inside_trim() {
    // The blue square at local (296.95, -205.476, 9.222, 7.853)
    // should land inside TrimBox -- left edge ~641.51 < 642.
    let ctm = Matrix::from_values(1.02883, 0.0, 0.0, -1.03942, 336.0, 426.0);
    let blue_rect = Rect::new(296.95, -205.476, 9.222, 7.853);
    let in_page_space = ctm.transform_rect(&blue_rect);
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);

    assert!(
        !in_page_space.is_outside(&trim),
        "blue rect left edge {:.4} should be inside trim right 642",
        in_page_space.x
    );
    println!("Blue rect transformed left edge: {:.4}", in_page_space.x);
    println!("\n");
}

#[test]
fn matrix_concat_compounds_correctly() {
    // Two pure translations should add together
    let t1 = Matrix::from_values(1.0, 0.0, 0.0, 1.0, 10.0, 20.0);
    let t2 = Matrix::from_values(1.0, 0.0, 0.0, 1.0, 5.0, 3.0);
    let combined = t1.concat(&t2);
    let (x, y) = combined.transform_point(0.0, 0.0);
    assert!((x - 15.0).abs() < 1e-10, "expected x=15, got {x}");
    assert!((y - 23.0).abs() < 1e-10, "expected y=23, got {y}");
    println!("Concatenated translation moves point to: ({}, {})", x, y);
    println!("\n");
}

#[test]
fn rect_is_outside_detects_all_four_directions() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);

    // Entirely to the right
    assert!(Rect::new(650.0, 100.0, 10.0, 10.0).is_outside(&trim));
    // Entirely to the left
    assert!(Rect::new(0.0, 100.0, 10.0, 10.0).is_outside(&trim));
    // Entirely above
    assert!(Rect::new(100.0, 830.0, 10.0, 10.0).is_outside(&trim));
    // Entirely below
    assert!(Rect::new(100.0, 0.0, 10.0, 10.0).is_outside(&trim));
    // Straddling from inside -- must NOT be outside
    assert!(!Rect::new(635.0, 100.0, 20.0, 10.0).is_outside(&trim));

    println!("Rect is_outside correctly detects all four directions.");
    println!("\n");
}

// -- Step 4 helper tests ----------------------------------------------

// Test helpers for building synthetic operations
fn op(operator: &str, operands: Vec<lopdf::Object>) -> Operation {
    Operation {
        operator: operator.to_string(),
        operands,
    }
}
fn real(v: f64) -> lopdf::Object {
    lopdf::Object::Real(v as f32)
}
fn int(v: i64) -> lopdf::Object {
    lopdf::Object::Integer(v)
}
fn name(n: &str) -> lopdf::Object {
    lopdf::Object::Name(n.as_bytes().to_vec())
}

#[test]
fn object_to_f64_converts_integer() {
    assert_eq!(object_to_f64(&int(42)), 42.0);
    assert_eq!(object_to_f64(&int(-7)), -7.0);
}

#[test]
fn object_to_f64_converts_real() {
    // lopdf stores Real as f32, so round-trip f64->f32->f64 loses ~1e-7 precision
    assert!((object_to_f64(&real(3.14)) - 3.14).abs() < 1e-5);
    assert!((object_to_f64(&real(-0.5)) + 0.5).abs() < 1e-5);
}

#[test]
#[should_panic(expected = "expected numeric Object")]
fn object_to_f64_panics_on_non_numeric() {
    object_to_f64(&name("X1"));
}

#[test]
fn operands_to_rect_positive_dimensions() {
    // re operands: x=10, y=20, w=100, h=50
    let operands = vec![real(10.0), real(20.0), real(100.0), real(50.0)];
    let r = operands_to_rect(&operands);
    assert!((r.x - 10.0).abs() < 1e-10);
    assert!((r.y - 20.0).abs() < 1e-10);
    assert!((r.width - 100.0).abs() < 1e-10);
    assert!((r.height - 50.0).abs() < 1e-10);
}

#[test]
fn operands_to_rect_negative_height() {
    // PDF allows negative height -- rect extends downward
    // re operands: x=10, y=80, w=100, h=-50 -> corners (10,80) and (110,30)
    let operands = vec![real(10.0), real(80.0), real(100.0), real(-50.0)];
    let r = operands_to_rect(&operands);
    // from_corners normalizes: x=10, y=30, w=100, h=50
    assert!((r.x - 10.0).abs() < 1e-10);
    assert!((r.y - 30.0).abs() < 1e-10);
    assert!((r.width - 100.0).abs() < 1e-10);
    assert!((r.height - 50.0).abs() < 1e-10);
}

#[test]
fn re_is_outside_with_identity_ctm_outside() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let ctm = Matrix::identity();
    // Rect entirely to the right of trim
    let operands = vec![real(650.0), real(100.0), real(10.0), real(10.0)];
    assert!(re_is_outside(&operands, &ctm, &trim));
}

#[test]
fn re_is_outside_with_identity_ctm_inside() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let ctm = Matrix::identity();
    // Rect inside trim
    let operands = vec![real(100.0), real(100.0), real(50.0), real(50.0)];
    assert!(!re_is_outside(&operands, &ctm, &trim));
}

// -- Step 4 filter_operations unit tests (synthetic) -------------------

#[test]
fn filter_drops_everything_after_marked_content_ends() {
    // BDC ... EMC closes marked content -> everything after is dropped
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let ops = vec![
        op("BDC", vec![name("OC"), name("MC0")]),
        op("q", vec![]),
        op("Q", vec![]),
        op("EMC", vec![]),
        // These are "print marks" -- should all be dropped
        op("q", vec![]),
        op("re", vec![real(0.0), real(0.0), real(10.0), real(10.0)]),
        op("f", vec![]),
        op("Q", vec![]),
    ];
    let filtered = filter_operations(&ops, Some(trim));
    let operators: Vec<&str> = filtered.iter().map(|o| o.operator.as_str()).collect();
    // Should contain BDC, q, Q, EMC but nothing after EMC
    assert!(operators.contains(&"BDC"));
    assert!(operators.contains(&"EMC"));
    // The post-EMC ops should be gone
    assert_eq!(
        operators.iter().filter(|&&o| o == "re").count(),
        0,
        "re after EMC should be dropped"
    );
    println!("Post-marked-content ops dropped: {:?}", operators);
}

#[test]
fn filter_keeps_inside_re_f_pair() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    // A rect well inside trim, wrapped in q/Q with BDC/EMC
    let ops = vec![
        op("BDC", vec![name("OC"), name("MC0")]),
        op("BDC", vec![name("OC"), name("MC1")]),
        op("q", vec![]),
        op("re", vec![real(100.0), real(100.0), real(50.0), real(50.0)]),
        op("f", vec![]),
        op("Q", vec![]),
        op("EMC", vec![]),
        op("EMC", vec![]),
    ];
    let filtered = filter_operations(&ops, Some(trim));
    let operators: Vec<&str> = filtered.iter().map(|o| o.operator.as_str()).collect();
    assert!(operators.contains(&"re"), "inside re should be kept");
    assert!(operators.contains(&"f"), "inside f should be kept");
    println!("Inside re+f pair kept: {:?}", operators);
}

#[test]
fn filter_drops_outside_re_f_pair() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    // A rect entirely to the right of trim
    let ops = vec![
        op("BDC", vec![name("OC"), name("MC0")]),
        op("BDC", vec![name("OC"), name("MC1")]),
        op("q", vec![]),
        op("re", vec![real(650.0), real(100.0), real(10.0), real(10.0)]),
        op("f", vec![]),
        op("Q", vec![]),
        op("EMC", vec![]),
        op("EMC", vec![]),
    ];
    let filtered = filter_operations(&ops, Some(trim));
    let operators: Vec<&str> = filtered.iter().map(|o| o.operator.as_str()).collect();
    assert_eq!(
        operators.iter().filter(|&&o| o == "re").count(),
        0,
        "outside re should be dropped"
    );
    assert_eq!(
        operators.iter().filter(|&&o| o == "f").count(),
        0,
        "outside f should be dropped"
    );
    println!("Outside re+f pair dropped: {:?}", operators);
}

#[test]
fn filter_drops_outside_image_block() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    // An image block where cm places origin at x=700 (outside trim right 642)
    let ops = vec![
        op("BDC", vec![name("OC"), name("MC0")]),
        op("BDC", vec![name("OC"), name("MC1")]),
        op("q", vec![]),
        op(
            "cm",
            vec![
                real(100.0),
                real(0.0),
                real(0.0),
                real(100.0),
                real(700.0),
                real(400.0),
            ],
        ),
        op("Do", vec![name("X1")]),
        op("Q", vec![]),
        op("EMC", vec![]),
        op("EMC", vec![]),
    ];
    let filtered = filter_operations(&ops, Some(trim));
    let operators: Vec<&str> = filtered.iter().map(|o| o.operator.as_str()).collect();
    assert!(
        !operators.contains(&"Do"),
        "outside image Do should be dropped"
    );
    println!("Outside image block dropped: {:?}", operators);
}

#[test]
fn filter_keeps_inside_image_block() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    // Image block with cm placing origin at x=300 (inside trim)
    let ops = vec![
        op("BDC", vec![name("OC"), name("MC0")]),
        op("BDC", vec![name("OC"), name("MC1")]),
        op("q", vec![]),
        op(
            "cm",
            vec![
                real(100.0),
                real(0.0),
                real(0.0),
                real(100.0),
                real(300.0),
                real(400.0),
            ],
        ),
        op("Do", vec![name("X1")]),
        op("Q", vec![]),
        op("EMC", vec![]),
        op("EMC", vec![]),
    ];
    let filtered = filter_operations(&ops, Some(trim));
    let operators: Vec<&str> = filtered.iter().map(|o| o.operator.as_str()).collect();
    assert!(operators.contains(&"Do"), "inside image Do should be kept");
    println!("Inside image block kept: {:?}", operators);
}

#[test]
fn filter_mixed_block_keeps_inside_drops_outside() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    // One inside rect and one outside rect in the same q/Q block
    let ops = vec![
        op("BDC", vec![name("OC"), name("MC0")]),
        op("BDC", vec![name("OC"), name("MC1")]),
        op("q", vec![]),
        // Inside rect
        op("re", vec![real(100.0), real(100.0), real(50.0), real(50.0)]),
        op("f", vec![]),
        // Outside rect (to the right)
        op("re", vec![real(650.0), real(100.0), real(10.0), real(10.0)]),
        op("f", vec![]),
        op("Q", vec![]),
        op("EMC", vec![]),
        op("EMC", vec![]),
    ];
    let filtered = filter_operations(&ops, Some(trim));
    let operators: Vec<&str> = filtered.iter().map(|o| o.operator.as_str()).collect();
    // Exactly 1 re and 1 f should remain (the inside pair)
    assert_eq!(
        operators.iter().filter(|&&o| o == "re").count(),
        1,
        "only inside re should survive"
    );
    assert_eq!(
        operators.iter().filter(|&&o| o == "f").count(),
        1,
        "only inside f should survive"
    );
    println!("Mixed block filtered correctly: {:?}", operators);
}

#[test]
fn filter_nested_q_blocks_handled() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    // Nested q/Q: outer contains inner with an outside image
    let ops = vec![
        op("BDC", vec![name("OC"), name("MC0")]),
        op("BDC", vec![name("OC"), name("MC1")]),
        op("q", vec![]),
        // Inner block: inside rect
        op("q", vec![]),
        op("re", vec![real(100.0), real(100.0), real(50.0), real(50.0)]),
        op("f", vec![]),
        op("Q", vec![]),
        // Inner block: outside image
        op("q", vec![]),
        op(
            "cm",
            vec![
                real(100.0),
                real(0.0),
                real(0.0),
                real(100.0),
                real(700.0),
                real(400.0),
            ],
        ),
        op("Do", vec![name("X1")]),
        op("Q", vec![]),
        op("Q", vec![]),
        op("EMC", vec![]),
        op("EMC", vec![]),
    ];
    let filtered = filter_operations(&ops, Some(trim));
    let operators: Vec<&str> = filtered.iter().map(|o| o.operator.as_str()).collect();
    // Inside rect should survive
    assert!(
        operators.contains(&"re"),
        "inside re in nested block should survive"
    );
    // Outside image should be dropped
    assert!(
        !operators.contains(&"Do"),
        "outside Do in nested block should be dropped"
    );
    println!("Nested q/Q blocks handled: {:?}", operators);
}

#[test]
fn filter_empty_input_returns_empty() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let filtered = filter_operations(&[], Some(trim));
    assert!(filtered.is_empty());
}

#[test]
fn filter_no_marked_content_keeps_all_inside() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    // No BDC/EMC -- mc_depth never goes positive so past_marked_content never flips
    let ops = vec![
        op("q", vec![]),
        op("re", vec![real(100.0), real(100.0), real(50.0), real(50.0)]),
        op("f", vec![]),
        op("Q", vec![]),
    ];
    let filtered = filter_operations(&ops, Some(trim));
    let operators: Vec<&str> = filtered.iter().map(|o| o.operator.as_str()).collect();
    assert_eq!(operators, vec!["q", "re", "f", "Q"]);
}

// -- Step 4 block_is_outside_image unit tests --------------------------

#[test]
fn block_is_outside_image_detects_outside() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let base_ctm = Matrix::identity();
    let block = vec![
        op("q", vec![]),
        op(
            "cm",
            vec![
                real(100.0),
                real(0.0),
                real(0.0),
                real(100.0),
                real(700.0),
                real(400.0),
            ],
        ),
        op("Do", vec![name("X1")]),
        op("Q", vec![]),
    ];
    assert!(block_is_outside_image(&block, &base_ctm, Some(&trim)));
}

#[test]
fn block_is_outside_image_keeps_inside() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let base_ctm = Matrix::identity();
    let block = vec![
        op("q", vec![]),
        op(
            "cm",
            vec![
                real(100.0),
                real(0.0),
                real(0.0),
                real(100.0),
                real(300.0),
                real(400.0),
            ],
        ),
        op("Do", vec![name("X1")]),
        op("Q", vec![]),
    ];
    assert!(!block_is_outside_image(&block, &base_ctm, Some(&trim)));
}

#[test]
fn block_is_outside_image_no_do_returns_false() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let base_ctm = Matrix::identity();
    // Block with no Do -- not an image block
    let block = vec![
        op("q", vec![]),
        op("re", vec![real(100.0), real(100.0), real(10.0), real(10.0)]),
        op("f", vec![]),
        op("Q", vec![]),
    ];
    assert!(!block_is_outside_image(&block, &base_ctm, Some(&trim)));
}

// -- Step 4 remove_outside_re_f_pairs unit tests -----------------------

#[test]
fn remove_outside_re_f_keeps_inside_pair() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let base_ctm = Matrix::identity();
    let block = vec![
        op("q", vec![]),
        op("re", vec![real(100.0), real(100.0), real(50.0), real(50.0)]),
        op("f", vec![]),
        op("Q", vec![]),
    ];
    let result = remove_outside_re_f_pairs(block, &base_ctm, Some(&trim));
    let operators: Vec<&str> = result.iter().map(|o| o.operator.as_str()).collect();
    assert_eq!(operators, vec!["q", "re", "f", "Q"]);
}

#[test]
fn remove_outside_re_f_drops_outside_pair() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let base_ctm = Matrix::identity();
    let block = vec![
        op("q", vec![]),
        op("re", vec![real(650.0), real(100.0), real(10.0), real(10.0)]),
        op("f", vec![]),
        op("Q", vec![]),
    ];
    let result = remove_outside_re_f_pairs(block, &base_ctm, Some(&trim));
    let operators: Vec<&str> = result.iter().map(|o| o.operator.as_str()).collect();
    assert_eq!(operators, vec!["q", "Q"], "outside re+f should be removed");
}

#[test]
fn remove_outside_re_f_keeps_re_not_followed_by_f() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    let base_ctm = Matrix::identity();
    // re followed by W (clip), not f -- should be kept regardless of position
    let block = vec![
        op("q", vec![]),
        op("re", vec![real(650.0), real(100.0), real(10.0), real(10.0)]),
        op("W", vec![]),
        op("n", vec![]),
        op("Q", vec![]),
    ];
    let result = remove_outside_re_f_pairs(block, &base_ctm, Some(&trim));
    let operators: Vec<&str> = result.iter().map(|o| o.operator.as_str()).collect();
    assert!(
        operators.contains(&"re"),
        "re+W should not be removed by re+f filter"
    );
}

#[test]
fn remove_outside_re_f_respects_ctm() {
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);
    // base_ctm translates x by +600 -- so a rect at local x=50 lands at page x=650 (outside)
    let base_ctm = Matrix::from_values(1.0, 0.0, 0.0, 1.0, 600.0, 0.0);
    let block = vec![
        op("q", vec![]),
        op("re", vec![real(50.0), real(100.0), real(10.0), real(10.0)]),
        op("f", vec![]),
        op("Q", vec![]),
    ];
    let result = remove_outside_re_f_pairs(block, &base_ctm, Some(&trim));
    let operators: Vec<&str> = result.iter().map(|o| o.operator.as_str()).collect();
    assert_eq!(
        operators,
        vec!["q", "Q"],
        "CTM should shift rect outside trim"
    );
}

// -- Step 4 integration test with actual PDF ---------------------------

#[test]
fn filter_operations_on_source_pdf_reduces_ops() {
    let doc = load_document(&test_asset(SOURCE_PDF_REL));
    let page_id = doc.get_pages()[&1];
    let content = doc.get_and_decode_page_content(page_id).unwrap();
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);

    let before = content.operations.len();
    let filtered = filter_operations(&content.operations, Some(trim));
    let after = filtered.len();

    println!("Before: {} ops, After: {} ops", before, after);
    assert!(
        after < before,
        "filtering should reduce operation count (before={before}, after={after})"
    );
}

#[test]
fn filter_operations_on_source_pdf_keeps_do_operator() {
    // The two inside image strips should survive -- at least one Do must remain
    let doc = load_document(&test_asset(SOURCE_PDF_REL));
    let page_id = doc.get_pages()[&1];
    let content = doc.get_and_decode_page_content(page_id).unwrap();
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);

    let filtered = filter_operations(&content.operations, Some(trim));
    let do_count = filtered.iter().filter(|o| o.operator == "Do").count();
    println!("Do operators remaining: {}", do_count);
    assert!(
        do_count >= 1,
        "at least one image Do should survive filtering"
    );
}

#[test]
fn filter_operations_on_source_pdf_no_ops_after_last_emc() {
    // After the final EMC in the filtered output, there should be no drawing ops
    let doc = load_document(&test_asset(SOURCE_PDF_REL));
    let page_id = doc.get_pages()[&1];
    let content = doc.get_and_decode_page_content(page_id).unwrap();
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);

    let filtered = filter_operations(&content.operations, Some(trim));
    // Find the last EMC
    let last_emc_idx = filtered.iter().rposition(|o| o.operator == "EMC");

    if let Some(idx) = last_emc_idx {
        let after_emc: Vec<&str> = filtered[idx + 1..]
            .iter()
            .map(|o| o.operator.as_str())
            .filter(|&op| op != "q" && op != "Q")
            .collect();
        assert!(
            after_emc.is_empty(),
            "no drawing ops should remain after final EMC, found: {:?}",
            after_emc
        );
        println!("No drawing ops after final EMC (only q/Q bookkeeping).");
    }
}

#[test]
#[allow(non_snake_case)]
fn filter_operations_on_source_pdf_q_Q_balanced() {
    // q and Q counts must match in the filtered output
    let doc = load_document(&test_asset(SOURCE_PDF_REL));
    let page_id = doc.get_pages()[&1];
    let content = doc.get_and_decode_page_content(page_id).unwrap();
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);

    let filtered = filter_operations(&content.operations, Some(trim));
    let q_count = filtered.iter().filter(|o| o.operator == "q").count();
    let big_q_count = filtered.iter().filter(|o| o.operator == "Q").count();
    println!("q count: {}, Q count: {}", q_count, big_q_count);
    assert_eq!(
        q_count, big_q_count,
        "q and Q must be balanced in filtered output"
    );
}

// -- Output constraint stubs (filled in after step 6) -----------------

/// Helper: run process_pdf and return the output path (temp file).
fn run_pipeline(test_name: &str) -> std::path::PathBuf {
    let out = std::env::temp_dir().join(format!("pdf_trim_test_{}.pdf", test_name));
    process_pdf(&test_asset(SOURCE_PDF_REL), &out).expect("process_pdf failed");
    out
}

// -- Step 5-8 integration / validation tests --------------------------

#[test]
fn remove_objects_outside_trim_box() {
    let out = run_pipeline("remove_objects");
    let doc = load_document(&out);
    let page_id = doc.get_pages()[&1];
    let content = doc.get_and_decode_page_content(page_id).unwrap();
    let trim = Rect::from_corners(30.0, 30.0, 642.0, 822.0);

    // Output should have fewer ops than source
    let source = load_document(&test_asset(SOURCE_PDF_REL));
    let src_page = source.get_pages()[&1];
    let src_content = source.get_and_decode_page_content(src_page).unwrap();
    assert!(
        content.operations.len() < src_content.operations.len(),
        "output ops ({}) should be fewer than source ops ({})",
        content.operations.len(),
        src_content.operations.len()
    );

    // Verify no remaining re+f rectangles are outside trim box
    let mut ctm_stack: Vec<Matrix> = vec![Matrix::identity()];
    let ops = &content.operations;
    for i in 0..ops.len() {
        match ops[i].operator.as_str() {
            "q" => ctm_stack.push(*ctm_stack.last().unwrap()),
            "Q" => {
                ctm_stack.pop();
            }
            "cm" => {
                let m = operands_to_matrix(&ops[i].operands);
                let top = ctm_stack.last_mut().unwrap();
                *top = top.concat(&m);
            }
            "re" => {
                // Check if next op is f/F/f* -- if so, rect must not be outside
                if i + 1 < ops.len() {
                    let next = ops[i + 1].operator.as_str();
                    if next == "f" || next == "F" || next == "f*" {
                        let ctm = ctm_stack.last().unwrap();
                        assert!(
                            !re_is_outside(&ops[i].operands, ctm, &trim),
                            "output still has rect outside trim at op {}",
                            i
                        );
                    }
                }
            }
            _ => {}
        }
    }
    println!("Output has {} ops, all re+f rects inside trim.", ops.len());
}

#[test]
fn ensure_no_white_rectangles() {
    // Constraint: we must NOT cover deleted areas with white rectangles.
    // Scan for the pattern: set fill to white ("1 g" or "1 1 1 rg") then re + f.
    let out = run_pipeline("no_white_rects");
    let doc = load_document(&out);
    let page_id = doc.get_pages()[&1];
    let content = doc.get_and_decode_page_content(page_id).unwrap();
    let ops = &content.operations;

    let mut fill_is_white = false;
    for i in 0..ops.len() {
        match ops[i].operator.as_str() {
            "g" => {
                // "1 g" sets grayscale fill to white
                if ops[i].operands.len() == 1 {
                    let v = object_to_f64(&ops[i].operands[0]);
                    fill_is_white = (v - 1.0).abs() < 1e-6;
                }
            }
            "rg" => {
                // "1 1 1 rg" sets RGB fill to white
                if ops[i].operands.len() == 3 {
                    let r = object_to_f64(&ops[i].operands[0]);
                    let g = object_to_f64(&ops[i].operands[1]);
                    let b = object_to_f64(&ops[i].operands[2]);
                    fill_is_white =
                        (r - 1.0).abs() < 1e-6 && (g - 1.0).abs() < 1e-6 && (b - 1.0).abs() < 1e-6;
                }
            }
            "f" | "F" | "f*" => {
                // If fill is white and previous op was "re", that's a white rect cover-up
                if fill_is_white && i > 0 && ops[i - 1].operator == "re" {
                    panic!(
                        "Found white-filled rectangle at ops[{}..{}] -- this violates \
                             the constraint against covering objects with white rects",
                        i - 1,
                        i
                    );
                }
            }
            // Any other fill-color operator resets the flag
            "k" | "K" | "G" | "RG" | "sc" | "SC" | "scn" | "SCN" => {
                fill_is_white = false;
            }
            _ => {}
        }
    }
    println!("No white-rectangle cover-ups found in output.");
}

#[test]
fn ensure_cropping_not_used() {
    // Constraint: we must NOT crop -- MediaBox and CropBox must be unchanged.
    let out = run_pipeline("no_cropping");
    let source = load_document(&test_asset(SOURCE_PDF_REL));
    let output = load_document(&out);

    let src_page_id = source.get_pages()[&1];
    let out_page_id = output.get_pages()[&1];

    let src_page = source.get_dictionary(src_page_id).unwrap();
    let out_page = output.get_dictionary(out_page_id).unwrap();

    // MediaBox must be identical
    let src_media = src_page.get(b"MediaBox").unwrap();
    let out_media = out_page.get(b"MediaBox").unwrap();
    assert_eq!(
        format!("{:?}", src_media),
        format!("{:?}", out_media),
        "MediaBox was changed -- cropping is not allowed"
    );

    // TrimBox must be identical
    let src_trim = src_page.get(b"TrimBox").unwrap();
    let out_trim = out_page.get(b"TrimBox").unwrap();
    assert_eq!(
        format!("{:?}", src_trim),
        format!("{:?}", out_trim),
        "TrimBox was changed -- this should remain untouched"
    );

    // If CropBox exists in source, it must be unchanged in output
    if let Ok(src_crop) = src_page.get(b"CropBox") {
        let out_crop = out_page
            .get(b"CropBox")
            .expect("CropBox removed from output");
        assert_eq!(
            format!("{:?}", src_crop),
            format!("{:?}", out_crop),
            "CropBox was changed -- cropping is not allowed"
        );
    }
    println!("No cropping detected: MediaBox, TrimBox, CropBox all unchanged.");
}

#[test]
fn final_pdf_validation() {
    let out = run_pipeline("final_validation");
    let output = load_document(&out);
    let goal = load_document(&test_asset(GOAL_PDF_REL));

    // Object count: output should be in the same ballpark as goal
    let out_obj_count = output.objects.len();
    let goal_obj_count = goal.objects.len();
    println!(
        "Output objects: {}, Goal objects: {}",
        out_obj_count, goal_obj_count
    );
    // Allow some variance -- our pruning may differ slightly from manually created goal
    let diff = (out_obj_count as isize - goal_obj_count as isize).unsigned_abs();
    assert!(
        diff <= 5,
        "output object count ({}) too far from goal ({})",
        out_obj_count,
        goal_obj_count
    );

    // Content stream op counts should be in the same ballpark
    let out_page = output.get_pages()[&1];
    let goal_page = goal.get_pages()[&1];
    let out_content = output.get_and_decode_page_content(out_page).unwrap();
    let goal_content = goal.get_and_decode_page_content(goal_page).unwrap();
    println!(
        "Output ops: {}, Goal ops: {}",
        out_content.operations.len(),
        goal_content.operations.len()
    );
    let op_diff = (out_content.operations.len() as isize - goal_content.operations.len() as isize)
        .unsigned_abs();
    assert!(
        op_diff <= 15,
        "output op count ({}) too far from goal ({})",
        out_content.operations.len(),
        goal_content.operations.len()
    );

    // Key operators in goal must also appear in output
    let out_op_set: std::collections::HashSet<&str> = out_content
        .operations
        .iter()
        .map(|o| o.operator.as_str())
        .collect();
    for goal_op in goal_content.operations.iter().map(|o| o.operator.as_str()) {
        assert!(
            out_op_set.contains(goal_op),
            "goal operator {:?} missing from output",
            goal_op
        );
    }

    // Both should have the same Do count (images kept)
    let out_do = out_content
        .operations
        .iter()
        .filter(|o| o.operator == "Do")
        .count();
    let goal_do = goal_content
        .operations
        .iter()
        .filter(|o| o.operator == "Do")
        .count();
    assert_eq!(out_do, goal_do, "Do operator count should match goal");
    println!(
        "Final validation passed: objects within range, key operators present, Do count matches."
    );
}
