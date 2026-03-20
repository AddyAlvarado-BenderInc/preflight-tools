use crate::matrix::Matrix;
use crate::rect::Rect;

pub type Operation = lopdf::content::Operation;

/// Converts a PDF object to a floating-point number.
///
/// This function extracts numeric values from PDF objects and converts them to `f64`.
/// It supports both integer and real number objects from the lopdf library.
///
/// # Arguments
///
/// * `object` - A reference to a `lopdf::Object` that should contain a numeric value
///
/// # Returns
///
/// * `f64` - The numeric value extracted from the PDF object
///
/// # Panics
///
/// This function will panic if the provided object is not a numeric type
/// (neither `lopdf::Object::Integer` nor `lopdf::Object::Real`).
///
/// # Examples
///
/// ```rust
/// use lopdf::Object;
///
/// let int_obj = Object::Integer(42);
/// let real_obj = Object::Real(3.14);
///
/// assert_eq!(object_to_f64(&int_obj), 42.0);
/// assert_eq!(object_to_f64(&real_obj), 3.14);
/// ```
pub fn object_to_f64(object: &lopdf::Object) -> f64 {
    match object {
        lopdf::Object::Integer(i) => *i as f64,
        lopdf::Object::Real(r) => *r as f64,
        _ => panic!("expected numeric Object, got {:?}", object),
    }
}

/// Converts a slice of PDF objects representing transformation matrix operands into a Matrix struct.
///
/// This function takes exactly 6 operands from a PDF transformation matrix and converts them
/// into a Matrix representation. The operands are expected to be in the order:
/// [a, b, c, d, e, f] which corresponds to the transformation matrix:
///
/// | a  b  0 |
/// | c  d  0 |
/// | e  f  1 |
///
/// # Arguments
///
/// * `operands` - A slice of lopdf::Object references containing exactly 6 numeric values
///
/// # Returns
///
/// A Matrix struct initialized with the 6 transformation values
///
/// # Panics
///
/// This function will panic if:
/// - The operands slice contains fewer than 6 elements
/// - Any operand cannot be converted to f64 via object_to_f64
pub(crate) fn operands_to_matrix(operands: &[lopdf::Object]) -> Matrix {
    let vals: Vec<f64> = operands.iter().map(object_to_f64).collect();
    Matrix::from_values(vals[0], vals[1], vals[2], vals[3], vals[4], vals[5])
}

/// Converts a slice of PDF operands to a rectangle representation.
///
/// This function takes a slice of `lopdf::Object` operands and converts them into
/// a `Rect` structure. The operands are expected to represent a rectangle in the
/// format [x, y, width, height] as defined by the PDF specification.
///
/// # Arguments
///
/// * `operands` - A slice of `lopdf::Object` containing exactly 4 numeric values
///   representing the rectangle coordinates and dimensions
///
/// # Returns
///
/// A `Rect` structure created from the corner coordinates derived from the operands.
///
/// # Details
///
/// The PDF rectangle operands are interpreted as:
/// * `x` - The x-coordinate of the rectangle's origin
/// * `y` - The y-coordinate of the rectangle's origin
/// * `width` - The width of the rectangle
/// * `height` - The height of the rectangle (can be negative, indicating the rectangle extends downward)
///
/// The function converts all operands to `f64` values and creates a rectangle using
/// `Rect::from_corners()` method.
///
/// # Panics
///
/// This function will panic if:
/// * The operands slice doesn't contain exactly 4 elements
/// * Any operand cannot be converted to a numeric f64 value
/// * The `Rect::from_corners()` method fails (depends on implementation)
pub(crate) fn operands_to_rect(operands: &[lopdf::Object]) -> Rect {
    let vals: Vec<f64> = operands.iter().map(object_to_f64).collect();
    // re operands are: x y width height
    // height can be negative (PDF allows it – means rect goes downward)
    Rect::from_corners(vals[0], vals[1], vals[0] + vals[2], vals[1] + vals[3])
}

/// Returns true if the rect defined by this `re` operation, transformed
/// by the given CTM, is outside the trim box
pub(crate) fn re_is_outside(operands: &[lopdf::Object], ctm: &Matrix, trim: &Rect) -> bool {
    let local_rect = operands_to_rect(operands);
    let page_rect = ctm.transform_rect(&local_rect);
    page_rect.is_outside(trim)
}

/// Filters a slice of PDF operations, removing any graphical elements that fall outside
/// the specified trimming rectangle (`trim`). This function processes the operations
/// while respecting the structure imposed by `q`/`Q` blocks (save/restore graphics state),
/// and handles marked content sections appropriately.
///
/// # Arguments
///
/// * `operations` - A slice of [`Operation`]s representing the PDF content stream.
/// * `trim` - A reference to a [`Rect`] defining the trimming area. Any drawing operation
///   fully outside this rectangle will be removed.
///
/// # Returns
///
/// A new vector of [`Operation`]s with out-of-bounds content filtered out,
/// preserving structural integrity such as `q`/`Q` nesting and marked content blocks.
///
/// # Details
///
/// The filtering logic works as follows:
///
/// - Operations are processed sequentially.
/// - Marked content sections (delimited by `BDC`/`BMC` and `EMC`) are tracked.
///   Once all marked content has been processed (i.e., after the final `EMC`),
///   subsequent operations considered print marks are dropped unconditionally.
/// - Graphics state is tracked using a CTM (Current Transformation Matrix) stack.
/// - Content within `q`/`Q` blocks is buffered until the corresponding `Q` is found.
///   Then, the whole block is analyzed:
///     - If all drawing operations are outside the trim box, the entire block is dropped.
///     - If some are inside and some outside, only the out-of-bounds draw commands are removed.
///     - If all are inside, the block is kept as-is.
/// - Non-drawing operations (e.g., color setting, line width) are preserved unless they
///   are part of an entirely discarded block.
///
/// # Note
///
/// This function assumes well-formed input where every `Q` matches a prior `q`,
/// and marked content blocks are properly closed.
pub fn filter_operations(operations: &[Operation], trim: &Rect) -> Vec<Operation> {
    let mut output: Vec<Operation> = Vec::new();

    let mut ctm_stack: Vec<Matrix> = vec![Matrix::identity()];

    // EMC depth counter – we count BDC/BMC pushes and EMC pops.
    // once this hits zero, we've exited all marked content sections – everything after this is print marks.
    // this then just drop the print marks unconditionally.
    let mut marked_content_depth: i32 = 0;
    // becomes true once we've passed the end of marked content.
    let mut past_marked_content: bool = false;

    // Buffer for the current q/Q block being evaluated.
    // None means we are at the top level (no open q block).
    // Each entry is (buffer_of_ops, q_nesting_depth_when_opened).
    // We use a stack of buffers to handle nested q/Q correctly.
    let mut block_stack: Vec<Vec<Operation>> = Vec::new();

    for operation in operations {
        if past_marked_content {
            match operation.operator.as_str() {
                "q" => {
                    let last = ctm_stack.last().cloned().unwrap_or(Matrix::identity());
                    ctm_stack.push(last);
                }
                "Q" => {
                    if !ctm_stack.is_empty() {
                        ctm_stack.pop();
                    }
                }
                "cm" => {
                    let m = operands_to_matrix(&operation.operands);
                    if let Some(top) = ctm_stack.last_mut() {
                        *top = top.concat(&m);
                    } else {
                        // handling missing graphic state
                        ctm_stack.push(m);
                    }
                }
                _ => {}
            }
            continue;
        }

        match operation.operator.as_str() {
            "BDC" | "BMC" => marked_content_depth += 1,
            "EMC" => {
                marked_content_depth -= 1;
                let mut flush_blocks = false;

                if marked_content_depth == 0 {
                    past_marked_content = true;
                    flush_blocks = true;
                    // Flush whatever is buffered, then emit the EMC pair and stop.
                    // The two EMC operators themselves are structural and should be kept.
                    // We'll emit them below by falling through to normal handling.
                }

                if flush_blocks {
                    while let Some(mut block) = block_stack.pop() {
                        block.push(Operation {
                            operator: "Q".to_string(),
                            operands: vec![],
                        });
                        ctm_stack.pop();
                        let filtered = filter_block(block, trim, &ctm_stack);
                        if let Some(parent) = block_stack.last_mut() {
                            parent.extend(filtered);
                        } else {
                            output.extend(filtered);
                        }
                    }
                }
            }
            _ => {}
        }

        match operation.operator.as_str() {
            "q" => {
                ctm_stack.push(*ctm_stack.last().unwrap());
                block_stack.push(vec![operation.clone()]);
            }

            "Q" => {
                ctm_stack.pop();

                if let Some(mut block) = block_stack.pop() {
                    block.push(operation.clone());

                    // Scan the block: does it contain any outside-trim re+f?
                    // If ALL drawable content is outside → drop entire block.
                    // If MIXED → surgically remove outside-trim re f pairs.
                    // If all inside → flush as-is.
                    let filtered_block = filter_block(block, trim, &ctm_stack);

                    // Push filtered ops to the right place —
                    // either the parent block buffer or final output
                    if let Some(parent) = block_stack.last_mut() {
                        parent.extend(filtered_block);
                    } else {
                        output.extend(filtered_block);
                    }
                }
            }

            "cm" => {
                let m = operands_to_matrix(&operation.operands);
                let top = ctm_stack.last_mut().unwrap();
                *top = top.concat(&m);

                if let Some(block) = block_stack.last_mut() {
                    block.push(operation.clone());
                } else {
                    output.push(operation.clone());
                }
            }

            // Literally everything else – just buffer or pass through
            _ => {
                if let Some(block) = block_stack.last_mut() {
                    block.push(operation.clone());
                } else {
                    output.push(operation.clone());
                }
            }
        }
    }

    output
}

/// Filters a block of operations by removing those that fall outside the specified trimming rectangle.
///
/// This function takes a vector of operations and filters out any operations that are determined
/// to be outside the bounds of the provided trimming rectangle, taking into account the current
/// transformation matrix (CTM) stack.
///
/// # Arguments
///
/// * `block` - A vector of `Operation` structs representing the operations to be filtered
/// * `trim` - A reference to a `Rect` defining the trimming boundaries
/// * `ctm_stack` - A slice of `Matrix` elements representing the current transformation matrix stack
///
/// # Returns
///
/// A new vector containing only the operations that fall within the trimming rectangle.
/// Returns an empty vector if the entire block is outside the image bounds.
///
/// # Process
///
/// 1. Determines the base transformation matrix from the CTM stack (uses identity matrix if stack is empty)
/// 2. Checks if the entire block is outside the image bounds - if so, returns empty vector
/// 3. Removes operation pairs that are outside the trimming rectangle while preserving the remaining operations
fn filter_block(block: Vec<Operation>, trim: &Rect, ctm_stack: &[Matrix]) -> Vec<Operation> {
    let base_ctm = ctm_stack.last().copied().unwrap_or(Matrix::identity());

    if block_is_outside_image(&block, &base_ctm, trim) {
        return vec![];
    }

    remove_outside_re_f_pairs(block, &base_ctm, trim)
}

/// Determines if a block of PDF operations is positioned outside the visible image area.
///
/// This function tracks the current transformation matrix (CTM) as it processes operations
/// in the block. It specifically looks for "Do" operations (XObject invocations) and checks
/// if their transformed position falls outside the right boundary of the trim rectangle.
///
/// # Arguments
///
/// * `block` - A slice of Operation structs representing PDF content stream operations
/// * `base_ctm` - The base transformation matrix to start with
/// * `trim` - A Rect defining the visible boundaries of the image
///
/// # Returns
///
/// Returns `true` if a "Do" operation is found and its x-coordinate (after transformation)
/// is greater than or equal to the right edge of the trim rectangle, indicating the content
/// is positioned outside the visible area. Returns `false` otherwise, including when no
/// "Do" operations are found or when they are within bounds.
///
/// # Logic
///
/// * Processes each operation in sequence, updating the local CTM
/// * For "cm" operations: updates the CTM by concatenating the matrix from operands
/// * For "Do" operations: transforms point (0,0) and checks if x-coordinate exceeds trim.right()
/// * Ignores all other operations
///
/// Note: The function currently only checks the right boundary and assumes (0,0) as the
/// reference point for XObject positioning.
pub(crate) fn block_is_outside_image(block: &[Operation], base_ctm: &Matrix, trim: &Rect) -> bool {
    let mut ctm_stack: Vec<Matrix> = vec![base_ctm.clone()];

    for operation in block {
        match operation.operator.as_str() {
            "q" => {
                let last = ctm_stack.last().cloned().unwrap_or(Matrix::identity());
                ctm_stack.push(last);
            }
            "Q" => {
                if !ctm_stack.is_empty() {
                    ctm_stack.pop();
                }
            }
            "cm" => {
                let m = operands_to_matrix(&operation.operands);
                if let Some(top) = ctm_stack.last_mut() {
                    *top = top.concat(&m)
                } else {
                    ctm_stack.push(m)
                }
            }
            "Do" => {
                let ctm = ctm_stack.last().unwrap();
                let (px, _py) = ctm.transform_point(0.0, 0.0);
                if px >= trim.right() {
                    return true;
                }
            }
            _ => {}
        }
    }
    false // has_do found but wasn't outside - don't drop
}

/// Removes rectangle fill pairs (`re` followed by `f` or `f*`) that are completely outside the specified trim area.
///
/// This function processes a block of PDF operations and removes any rectangle (`re`) operations
/// that are immediately followed by a fill operation (`f` or `f*`) when the rectangle is determined
/// to be completely outside the specified trim boundary. The coordinate transformation matrix (CTM)
/// is tracked throughout the process to properly transform rectangle coordinates for boundary checking.
///
/// # Arguments
///
/// * `block` - A vector of PDF operations to process
/// * `base_ctm` - The base coordinate transformation matrix to use for coordinate calculations
/// * `trim` - The rectangular boundary used to determine if rectangles are outside the visible area
///
/// # Returns
///
/// A new vector of operations with outside rectangle-fill pairs removed
///
/// # Processing Logic
///
/// * Tracks the current CTM by processing `cm` (concatenate matrix) operations
/// * Identifies `re` operations followed by `f` or `f*` operations
/// * Removes rectangle-fill pairs when the rectangle is completely outside the trim area
/// * Preserves all other operations unchanged
///
/// # Example
///
/// ```rust
/// // Operations: [cm(...), re(x,y,w,h), f, g(0.5), re(x,y,w,h), f]
/// // If first rect is outside trim and second is inside
/// // Result: [cm(...), g(0.5), re(x,y,w,h), f]
/// ```
pub(crate) fn remove_outside_re_f_pairs(
    block: Vec<Operation>,
    base_ctm: &Matrix,
    trim: &Rect,
) -> Vec<Operation> {
    let mut result: Vec<Operation> = Vec::new();
    let mut ctm_stack: Vec<Matrix> = vec![*base_ctm];
    let mut i = 0;

    while i < block.len() {
        let operation = &block[i];

        match operation.operator.as_str() {
            "q" => {
                let last = ctm_stack.last().copied().unwrap_or(Matrix::identity());
                ctm_stack.push(last);
                result.push(operation.clone());
                i += 1;
            }
            "Q" => {
                if !ctm_stack.is_empty() {
                    ctm_stack.pop();
                }
                result.push(operation.clone());
                i += 1;
            }
            "cm" => {
                let m = operands_to_matrix(&operation.operands);
                if let Some(top) = ctm_stack.last_mut() {
                    *top = top.concat(&m);
                } else {
                    ctm_stack.push(m);
                };
                result.push(operation.clone());
                i += 1;
            }
            "re" => {
                let next_operation = block.get(i + 1).map(|o| o.operator.as_str());
                if next_operation == Some("f") || next_operation == Some("f*") {
                    let local_ctm = ctm_stack.last().copied().unwrap_or(Matrix::identity());
                    if re_is_outside(&operation.operands, &local_ctm, trim) {
                        i += 2;
                        continue;
                    }
                }
                result.push(operation.clone());
                i += 1;
            }

            _ => {
                result.push(operation.clone());
                i += 1;
            }
        }
    }

    result
}
