use crate::filter::{Operation, filter_operations, object_to_f64};
use crate::rect::Rect;
use lopdf::Object;
use lopdf::content::Content;
use std::collections::HashSet;
use std::path::Path;

/// Processes a PDF file by trimming page contents based on trim boxes and saving the result.
///
/// This function loads a PDF document, processes each page to remove content outside
/// the trim box boundaries, and saves the modified document to a new file. The processing
/// involves decoding page content, filtering operations that fall outside the trim area,
/// re-encoding the content, and pruning unused resources.
///
/// # Arguments
///
/// * `input_path` - Path to the input PDF file to be processed
/// * `output_path` - Path where the processed PDF file will be saved
///
/// # Returns
///
/// * `Ok(())` - If the PDF was successfully processed and saved
/// * `Err(lopdf::Error)` - If any error occurs during PDF processing (loading, parsing, or saving)
///
/// # Processing Steps
///
/// 1. Loads the PDF document from the input path
/// 2. Iterates through each page in the document
/// 3. Retrieves the trim box for each page to determine visible boundaries
/// 4. Decodes the page content operations
/// 5. Filters out operations that fall outside the trim box
/// 6. Re-encodes the filtered content back into the page's content stream
/// 7. Updates the page's resources to only include those referenced by remaining operations
/// 8. Removes unused objects from the document
/// 9. Saves the processed document to the output path
///
/// # Example
///
/// ```rust
/// use std::path::Path;
/// use your_crate::process_pdf;
///
/// let input = Path::new("input.pdf");
/// let output = Path::new("output.pdf");
/// process_pdf(input, output)?;
/// ```
pub fn process_pdf(input_path: &Path, output_path: &Path) -> lopdf::Result<()> {
    let mut document = lopdf::Document::load(input_path)?;
    let pages: Vec<lopdf::ObjectId> = document.get_pages().into_iter().map(|(_, id)| id).collect();

    for page_id in &pages {
        let trim = get_trim_box(&document, *page_id);

        let content = document.get_and_decode_page_content(*page_id)?;
        let filtered_operations = filter_operations(&content.operations, &trim);

        let content_to_encode = Content {
            operations: &filtered_operations[..],
        };
        let encoded = content_to_encode.encode();

        let stream_id = document.get_page_contents(*page_id)[0];
        let stream = document.get_object_mut(stream_id)?.as_stream_mut();
        stream?.set_plain_content(encoded?);

        let referenced = collect_referenced_resources(&filtered_operations);
        prune_page_resources(&mut document, *page_id, &referenced)?;
    }

    document.prune_objects();

    document.save(output_path)?;
    Ok(())
}

fn collect_referenced_resources(operations: &[Operation]) -> HashSet<Vec<u8>> {
    let mut names = HashSet::new();
    for operation in operations {
        match operation.operator.as_str() {
            "gs" | "Do" | "cs" | "CS" | "scn" | "SCN" | "sh" => {
                if let Some(lopdf::Object::Name(n)) = operation.operands.first() {
                    names.insert(n.clone());
                }
            }
            "Tf" => {
                if let Some(lopdf::Object::Name(n)) = operation.operands.first() {
                    names.insert(n.clone());
                }
            }
            _ => {}
        }
    }
    names
}

fn prune_page_resources(
    document: &mut lopdf::Document,
    page_id: lopdf::ObjectId,
    referenced: &HashSet<Vec<u8>>,
) -> lopdf::Result<()> {
    let page = document.get_dictionary(page_id)?;
    let resources_obj = page.get(b"Resources")?;

    // Resources can be an indirect reference or an inline dictionary.
    let resources_id = match resources_obj {
        Object::Reference(id) => *id,
        Object::Dictionary(_) => {
            // Inline dict — it lives inside the page object itself.
            // We need the page object's id to mutate it later.
            page_id
        }
        _ => return Ok(()),
    };
    let is_inline = resources_id == page_id;

    // Pass 1: collect indirect sub-dict ObjectIds and inline keys (immutable borrow)
    let mut indirect_subs: Vec<lopdf::ObjectId> = Vec::new();
    let mut inline_keys: Vec<Vec<u8>> = Vec::new();
    {
        let resources = if is_inline {
            let page_dict = document.get_dictionary(page_id)?;
            page_dict.get(b"Resources")?.as_dict()?
        } else {
            document.get_dictionary(resources_id)?
        };
        for key in &[b"ExtGState" as &[u8], b"Font", b"XObject", b"ColorSpace"] {
            match resources.get(*key) {
                Ok(Object::Reference(sub_id)) => indirect_subs.push(*sub_id),
                Ok(Object::Dictionary(_)) => inline_keys.push(key.to_vec()),
                _ => {}
            }
        }
    }

    // Pass 2a: prune indirect sub-dictionaries
    for sub_id in indirect_subs {
        if let Ok(sub_dict) = document.get_object_mut(sub_id)?.as_dict_mut() {
            let to_remove: Vec<Vec<u8>> = sub_dict
                .iter()
                .filter(|(name, _)| !referenced.contains(*name))
                .map(|(name, _)| name.clone())
                .collect();
            for name in to_remove {
                sub_dict.remove(&name);
            }
        }
    }

    // Pass 2b: prune inline sub-dictionaries
    let resources_dict = if is_inline {
        let page_dict = document.get_object_mut(page_id)?.as_dict_mut()?;
        page_dict.get_mut(b"Resources")?.as_dict_mut()?
    } else {
        document.get_object_mut(resources_id)?.as_dict_mut()?
    };
    for key in &inline_keys {
        if let Ok(Object::Dictionary(sub_dict)) = resources_dict.get_mut(key.as_slice()) {
            let to_remove: Vec<Vec<u8>> = sub_dict
                .iter()
                .filter(|(name, _)| !referenced.contains(*name))
                .map(|(name, _)| name.clone())
                .collect();
            for name in to_remove {
                sub_dict.remove(&name);
            }
        }
    }
    Ok(())
}

fn get_trim_box(doc: &lopdf::Document, page_id: lopdf::ObjectId) -> Rect {
    let page = doc.get_dictionary(page_id).unwrap();
    let trim_box = page.get(b"TrimBox").unwrap().as_array().unwrap();
    let values: Vec<f64> = trim_box.iter().map(object_to_f64).collect();
    Rect::from_corners(values[0], values[1], values[2], values[3])
}
