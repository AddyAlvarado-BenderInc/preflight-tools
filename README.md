# PDF Print Mark Removal - WIP

Remove print marks from PDF files by genuinely deleting all content that falls
outside the page's **TrimBox** -- while preserving everything inside it.

## Constraints

| Rule              | Detail                                                                                              |
|-------------------|-----------------------------------------------------------------------------------------------------|
| **No cropping**   | MediaBox and CropBox dimensions stay unchanged.                                                     |
| **No covering**   | No white rectangles or overlay objects to simulate erasure.                                         |
| **True deletion** | Content is removed from the content stream and unused objects are pruned from the PDF object table. |

## Dependencies

- **Language:** Rust (2024 edition)
- **Crate:** [`lopdf 0.40.0`](https://crates.io/crates/lopdf)
  ([source](https://github.com/J-F-Liu/lopdf)) -- low-level PDF manipulation
- No paid services or external tooling required at runtime.

---

## How It Works

### High-level pipeline

1. **Load** the PDF with
   [`lopdf::Document::load`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.load).
2. **Enumerate pages** via
   [`Document::get_pages`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.get_pages),
   which returns a `BTreeMap<u32, ObjectId>` mapping page numbers to object IDs.
3. **Read the TrimBox** from the page dictionary
   ([`Document::get_dictionary`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.get_dictionary)
   and [`Dictionary::get`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Dictionary.html#method.get)).
   The TrimBox is a four-element array `[x0, y0, x1, y1]` stored as
   [`Object::Integer`](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html#variant.Integer)
   or [`Object::Real`](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html#variant.Real) values.
   See PDF Reference 1.7, Section 14.11.2 -- Page Boundaries.
4. **Decode the content stream** with
   [`Document::get_and_decode_page_content`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.get_and_decode_page_content),
   which returns a
   [`Content`](https://docs.rs/lopdf/0.40.0/lopdf/content/struct.Content.html)
   whose `operations` field is a `Vec<`[`Operation`](https://docs.rs/lopdf/0.40.0/lopdf/content/struct.Operation.html)`>`.
   Each `Operation` has an `operator: String` and `operands: Vec<Object>`.
5. **Filter operations** -- walk the operations while tracking a CTM
   (Current Transformation Matrix) stack:
   - Compute each drawing primitive's bounding box in page coordinates.
   - If the bounding box is **entirely outside** the TrimBox, mark it for deletion.
   - If any part overlaps or starts inside the TrimBox, keep it.
6. **Re-encode** the filtered operations via
   [`Content::encode`](https://docs.rs/lopdf/0.40.0/lopdf/content/struct.Content.html#method.encode)
   and write the bytes back into the page's content stream
   ([`Document::get_object_mut`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.get_object_mut)
   and [`Stream::set_plain_content`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Stream.html#method.set_plain_content)).
7. **Prune resources** -- collect all `/Name` references still present in
   the surviving operations, then remove unreferenced entries from the page's
   `/Resources` sub-dictionaries (`/ExtGState`, `/Font`, `/XObject`,
   `/ColorSpace`) using
   [`Dictionary::remove`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Dictionary.html#method.remove).
8. **Prune orphaned objects** via
   [`Document::prune_objects`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.prune_objects).
9. **Save** the modified PDF with
   [`Document::save`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.save).

### Content stream filtering

PDF drawing instructions are a flat token stream. Operators cannot be evaluated
in isolation -- an `re` (rectangle) only matters when paired with the `f` (fill)
or `S` (stroke) that follows it, and a `cm` (concat matrix) only affects
whatever comes after it within its `q`/`Q` (save/restore graphics state) block.

The filter processes operations inside buffered `q`/`Q` blocks:

- **Image blocks** (`cm` + `Do`): if the CTM-transformed origin lands outside
  the trim boundary, the entire block is dropped.
- **Rectangle fills** (`re` + `f`/`f*`): each pair is individually tested
  against the TrimBox. Outside pairs are removed; inside pairs are kept.
- **Marked content** (`BDC`/`BMC` ... `EMC`): the artwork lives inside nested
  marked content sections. Once all marked content closes (depth returns to 0),
  everything remaining in the stream is unconditionally a print mark and is
  dropped.

### Boundary rule

An object is **outside** if its bounding box is completely beyond any edge of
the TrimBox (left edge >= trim right, right edge <= trim left, etc.). Objects
that **straddle** the boundary from inside are kept. This correctly handles
sub-point edge cases (e.g., a rect whose left edge is at 641.51 vs. a trim
right of 642).

---

## PDF Background

This section provides context for contributors who are not familiar with the
PDF specification internals that this project relies on. For a thorough
understanding, see the [PDF Reference](#references) documents listed at the end
of this file.

### Page boxes

A PDF page can define several rectangles (in points, origin at bottom-left).
These are specified in PDF Reference 1.7, Section 14.11.2 (Page Boundaries):

| Box         | Meaning                                                                        |
|-------------|--------------------------------------------------------------------------------|
| MediaBox    | Full physical page, including all bleed/mark space.                            |
| **TrimBox** | The finished page boundary after cutting -- **this is the deletion boundary**. |
| BleedBox    | Bleed zone extending slightly beyond the TrimBox.                              |
| CropBox     | Visible page area (often same as MediaBox).                                    |

In `lopdf`, page boxes are read from the page
[`Dictionary`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Dictionary.html) via
[`Dictionary::get`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Dictionary.html#method.get),
which returns an [`Object`](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html).
Box values are `Object::Array` containing `Object::Integer` or `Object::Real`
elements.

### Content stream

The page's drawing instructions live in a content stream -- a flat sequence of
plain-text tokens in the pattern (PDF Reference 1.7, Section 7.8.2):

```
operand operand ... operand  OPERATOR
```

`lopdf` represents this as
[`Content`](https://docs.rs/lopdf/0.40.0/lopdf/content/struct.Content.html)`.operations`,
a `Vec<`[`Operation`](https://docs.rs/lopdf/0.40.0/lopdf/content/struct.Operation.html)`>`.
Each `Operation` carries:
- `operator: String` -- the PDF operator name (`q`, `cm`, `re`, `f`, etc.)
- `operands: Vec<`[`Object`](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html)`>` --
  the preceding values consumed by the operator

Key operators (PDF Reference 1.7, Section 8.2 -- Graphics Objects, and
Section 9 -- Text):

| Operator      | Operands       | Meaning                                     | Spec section |
|---------------|----------------|---------------------------------------------|--------------|
| `q` / `Q`     | none           | Push / pop graphics state (including CTM)   | 8.4.4        |
| `cm`          | `a b c d e f`  | Multiply the Current Transformation Matrix  | 8.4.4        |
| `re`          | `x y w h`      | Define a rectangle path                     | 8.5.2.1      |
| `f` / `f*`    | none           | Fill the current path                       | 8.5.3.1      |
| `S`           | none           | Stroke the current path                     | 8.5.3.1      |
| `n`           | none           | End path without painting                   | 8.5.3.1      |
| `W`           | none           | Set clipping path from current path         | 8.5.4        |
| `Do`          | `/Name`        | Draw a named XObject (image)                | 8.8          |
| `BT` / `ET`   | none           | Begin / end text block                      | 9.4          |
| `Tm`          | `a b c d e f`  | Set the text matrix (position)              | 9.4.2        |
| `TJ` / `Tj`   | array / string | Draw text                                   | 9.4.3        |
| `BDC` / `BMC` | varies         | Begin marked content                        | 14.6         |
| `EMC`         | none           | End marked content                          | 14.6         |
| `gs`          | `/Name`        | Apply a named graphics state from resources | 8.4.5        |

### CTM (Current Transformation Matrix)

Every coordinate in the content stream is in **local space**. The CTM maps
local coordinates to page space (PDF Reference 1.7, Section 8.3.2 --
Coordinate Spaces, and Section 8.3.4 -- Transformation Matrices):

```
x' = a*x + c*y + e
y' = b*x + d*y + f
```

The CTM is managed as a stack: `q` saves the current matrix, `Q` restores
it, and `cm` concatenates (multiplies) a new transformation onto the top.

### Marked content structure

The artwork in a typical pre-press PDF lives inside nested `BDC`/`EMC` pairs
(PDF Reference 1.7, Section 14.6 -- Marked Content). Everything after the
outermost `EMC` closes is a print mark (registration targets, color bars, slug
text, etc.):

```
q
  BDC               <- marked content depth = 1
    BDC             <- depth = 2
      ... artwork ...
    EMC             <- depth = 1
  EMC               <- depth = 0 -- everything after this is print marks
  ... print marks ...
Q
```

---

## Project Structure

```
Cargo.toml
README.md
src/
    lib.rs          -- crate root: module declarations and public re-exports
    main.rs         -- binary entry point
    rect.rs         -- Rect struct (bounding box math, inside/outside test)
    matrix.rs       -- Matrix struct (2D affine transforms, CTM operations)
    filter.rs       -- content stream filtering (the core algorithm)
    process.rs      -- top-level pipeline (load, filter, prune, save)
    tests.rs        -- unit and integration tests (compiled only in test builds)
test/
    test_assets/    -- PDF fixtures used by integration tests
    test_result/    -- output directory for test runs
```

### Key types and functions

| Item                | Location     | Purpose                                                              |
|---------------------|--------------|----------------------------------------------------------------------|
| `Rect`              | `rect.rs`    | Axis-aligned rectangle with `is_outside(trim)` boundary test.        |
| `Matrix`            | `matrix.rs`  | 2D affine matrix with `concat`, `transform_point`, `transform_rect`. |
| `filter_operations` | `filter.rs`  | Walks a content stream, drops out-of-bounds drawing operations.      |
| `process_pdf`       | `process.rs` | End-to-end pipeline: load PDF, filter pages, prune resources, save.  |
| `object_to_f64`     | `filter.rs`  | Converts `lopdf::Object` (Integer or Real) to `f64`.                 |
| `Operation`         | `filter.rs`  | Type alias for `lopdf::content::Operation`.                          |

### lopdf API surface used

This project uses a focused subset of `lopdf`. The table below maps each
`lopdf` item to where it appears in the codebase, for contributors who want to
understand the library integration.

| lopdf item | Kind | Used in | Purpose |
|---|---|---|---|
| [`Document`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html) | struct | `process.rs` | Top-level PDF document handle. |
| [`Document::load`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.load) | method | `process.rs` | Load a PDF from a file path. |
| [`Document::save`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.save) | method | `process.rs` | Write the modified PDF to disk. |
| [`Document::get_pages`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.get_pages) | method | `process.rs` | Get the page-number-to-ObjectId map. |
| [`Document::get_dictionary`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.get_dictionary) | method | `process.rs` | Read a page or object as a `Dictionary`. |
| [`Document::get_and_decode_page_content`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.get_and_decode_page_content) | method | `process.rs` | Decode a page's content stream into `Content`. |
| [`Document::get_page_contents`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.get_page_contents) | method | `process.rs` | Get the ObjectId of a page's content stream. |
| [`Document::get_object_mut`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.get_object_mut) | method | `process.rs` | Get a mutable reference to a PDF object. |
| [`Document::prune_objects`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html#method.prune_objects) | method | `process.rs` | Remove unreachable objects from the object table. |
| [`Content`](https://docs.rs/lopdf/0.40.0/lopdf/content/struct.Content.html) | struct | `process.rs` | Decoded content stream; holds `Vec<Operation>`. |
| [`Content::encode`](https://docs.rs/lopdf/0.40.0/lopdf/content/struct.Content.html#method.encode) | method | `process.rs` | Re-encode operations to raw bytes. |
| [`Operation`](https://docs.rs/lopdf/0.40.0/lopdf/content/struct.Operation.html) | struct | `filter.rs` | Single PDF operator with its operands. |
| [`Object`](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html) | enum | `filter.rs`, `process.rs` | PDF object (Integer, Real, Name, Array, Dictionary, Stream, Reference, etc.). |
| [`Object::Integer`](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html#variant.Integer) | variant | `filter.rs` | Integer operand (`i64`). |
| [`Object::Real`](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html#variant.Real) | variant | `filter.rs` | Floating-point operand (`f32`). |
| [`Object::Name`](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html#variant.Name) | variant | `process.rs` | PDF name (e.g., resource keys like `/GS7`). |
| [`Object::Reference`](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html#variant.Reference) | variant | `process.rs` | Indirect reference to another object. |
| [`Dictionary`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Dictionary.html) | struct | `process.rs` | PDF dictionary; used for page dicts and `/Resources`. |
| [`Dictionary::get`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Dictionary.html#method.get) | method | `process.rs` | Look up a key in a dictionary. |
| [`Dictionary::remove`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Dictionary.html#method.remove) | method | `process.rs` | Remove an entry from a dictionary. |
| [`Stream::set_plain_content`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Stream.html#method.set_plain_content) | method | `process.rs` | Replace a stream's decoded byte content. |
| [`ObjectId`](https://docs.rs/lopdf/0.40.0/lopdf/type.ObjectId.html) | type alias | `process.rs` | `(u32, u16)` -- object number and generation. |

---

## Usage

### Build

```bash
cargo build --release
```

### Run

```bash
cargo run --release -- <input.pdf> [output.pdf]
```

- `<input.pdf>` -- path to the PDF file to process (required).
- `[output.pdf]` -- path for the output file (optional). Defaults to
  `test/test_result/<input>_trimmed.pdf` relative to the project root.

### Test

```bash
cargo test -- --nocapture
```

The `--nocapture` flag prints diagnostic output during test runs. To run a
single test:

```bash
cargo test test_name -- --nocapture
```

---

## Reference Values

These values were derived from direct analysis of the test PDF using
`qpdf --qdf` and are encoded in the test suite.

| Item                   | Value                                      |
|------------------------|--------------------------------------------|
| TrimBox                | `[30, 30, 642, 822]`                       |
| PlacedPDF CTM          | `1.02883 0 0 -1.03942 336 426`             |
| Red rect 1 (local)     | `x=298.292, y=-312.455, w=7.879, h=60.394` |
| Red rect 1 (page x)    | ~642.89 -- outside trim right (642)        |
| Red rect 2 (local)     | `x=297.557, y=-247.772, w=7.432, h=24.256` |
| Red rect 2 (page x)    | ~642.14 -- outside trim right (642)        |
| Blue rect (local)      | `x=296.95, y=-205.476, w=9.222, h=7.853`   |
| Blue rect (page x)     | ~641.51 -- inside trim (< 642), kept       |
| Image strip 3 origin x | 642.097 -- outside trim, dropped           |

---

## PDF Inspection Tools

These tools are useful for debugging and verifying output:

```bash
# Decompress a PDF into human-readable form
qpdf --qdf --object-streams=disable input.pdf readable.pdf

# Show page boxes (MediaBox, TrimBox, BleedBox, etc.)
pdfinfo input.pdf

# List all embedded images with metadata
pdfimages -list input.pdf

# Structural integrity check
qpdf --check input.pdf
```

Install: `brew install qpdf poppler` (macOS) or `apt install qpdf poppler-utils` (Debian/Ubuntu).

---

## Contributing

Contributions are welcome. When making changes, keep the following in mind:

- Run the full test suite (`cargo test`) before submitting a pull request.
- The boundary rule (straddle = keep) is intentional and must not change.
- All page boxes (MediaBox, CropBox, TrimBox) must remain unmodified in the
  output -- the tool removes content, never adjusts geometry.
- Do not introduce cover-up strategies (white rectangles, etc.). Content must
  be genuinely deleted.

---

## References

### PDF specification

The PDF format is defined by a series of reference documents. The sections cited
throughout this README refer to the **PDF Reference 1.7** unless noted otherwise.
All of the following are freely available:

- **PDF Reference 1.7** (ISO 32000-1:2008) --
  [Adobe PDF Reference, Sixth Edition](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf).
  The primary reference used in this project. Key chapters:
  - Chapter 7 -- Syntax (objects, file structure, content streams)
  - Chapter 8 -- Graphics (coordinate systems, CTM, path construction, painting operators)
  - Chapter 9 -- Text (text state, fonts, text-showing operators)
  - Chapter 14.6 -- Marked Content
  - Chapter 14.11.2 -- Page Boundaries (MediaBox, TrimBox, BleedBox, CropBox)
- **PDF Reference 1.6** --
  [Adobe PDF Reference, Fifth Edition](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.6.pdf).
  Useful as a secondary reference; same chapter structure as 1.7.
- **PDF Reference 1.5** --
  [Adobe PDF Reference, Fourth Edition](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.5_v6.pdf).
- **PDF Reference 1.4** --
  [Adobe PDF Reference, Third Edition](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.4.pdf).
- **PDF Reference 1.3** --
  [Adobe PDF Reference, Second Edition](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.3.pdf).
  The oldest version; covers the foundational object model and graphics state
  that has remained stable through all subsequent versions.

### Adobe Acrobat SDK

- **Acrobat SDK Documentation** --
  [Adobe Acrobat SDK Docs](https://opensource.adobe.com/dc-acrobat-sdk-docs/).
  Overview page with links to all reference PDFs, the JavaScript API reference,
  and supplement documents for each PDF version.

### lopdf crate

- **Crate page** -- [crates.io/crates/lopdf](https://crates.io/crates/lopdf)
- **API documentation** -- [docs.rs/lopdf/0.40.0](https://docs.rs/lopdf/0.40.0/lopdf/)
- **Source repository** -- [github.com/J-F-Liu/lopdf](https://github.com/J-F-Liu/lopdf)

Key documentation entry points for contributors:
- [`Document`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Document.html) --
  the central struct for loading, querying, mutating, and saving PDFs.
- [`Object` enum](https://docs.rs/lopdf/0.40.0/lopdf/enum.Object.html) --
  all PDF object types (Integer, Real, Name, Array, Dictionary, Stream, Reference, etc.).
- [`content` module](https://docs.rs/lopdf/0.40.0/lopdf/content/index.html) --
  `Content` and `Operation` types for working with content streams.
- [`Dictionary`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Dictionary.html) --
  PDF dictionary access and mutation.
- [`Stream`](https://docs.rs/lopdf/0.40.0/lopdf/struct.Stream.html) --
  PDF stream objects (content streams, font programs, images).

### Tools

- **qpdf** -- [qpdf.sourceforge.io](https://qpdf.sourceforge.io/).
  Decompresses PDFs into human-readable QDF form; invaluable for inspecting
  structure.
- **Poppler utilities** -- [poppler.freedesktop.org](https://poppler.freedesktop.org/).
  Provides `pdfinfo`, `pdfimages`, and other CLI tools for PDF analysis.

---

## License

This project is licensed under the
[MIT License](https://opensource.org/licenses/MIT).

Copyright (c) 2026 Addy Alvarado

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

---

END

⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⠤⢔⣶⣖⢂⢒⡐⠢⠤⣀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⠤⢊⠵⠒⣩⠟⠛⠙⠂⠀⠀⠉⠒⢤⣾⣖⠤⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠀⠀⣀⡤⠄⣀⠀⠀⠀⠀⠀⢀⠔⡡⠊⠀⠀⠀⠁⣀⣀⠀⠀⠀⠀⠀⠀⠈⠉⠻⡆⠈⠢⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⢠⠋⠁⠀⠀⠈⠱⡄⠀⠀⡠⠃⡜⠀⠀⠀⠀⢀⣾⠗⠋⠛⢆⠀⠀⠀⣠⣤⣤⡄⠉⢢⠀⠑⠄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⢼⠀⠀⠀⠀⠀⠀⠱⠀⢠⠃⢠⠃⠀⠀⠀⠀⢸⠋⣠⣤⡀⠘⡆⠀⢰⡿⠋⠉⠳⣄⠈⣆⠀⠐⡄⠀⠀⢀⠔⠂⠐⠲⢄⠀⠀⠀
⠀⠀⠀⠈⢆⠀⠀⠀⠀⠀⢀⢃⠆⠀⠀⠁⠀⠀⢄⣀⣹⠀⣷⣼⣿⠀⢻⠀⢿⣖⣹⣷⡀⠈⡆⠈⠀⠀⢰⡀⠰⠃⠀⠀⠀⠀⠀⡇⠀⠀
⠀⠀⠀⠀⠈⣆⠤⠤⠤⠤⠾⣼⡀⠀⠀⠀⠀⠀⢀⡀⠂⠙⠻⡓⠋⢀⡏⠀⠀⢿⢿⡽⠃⠀⡜⠀⠀⠀⠀⡇⡇⠀⠀⠀⠀⠀⡠⠁⠀⠀
⠀⠀⢀⠔⡩⠀⠀⠀⠀⠀⠀⠀⠉⠓⢄⠀⠀⠊⠁⠙⢕⠂⠀⠘⡖⠊⠀⠀⠀⠀⠑⡤⠔⠊⡉⠐⠀⠀⢀⣰⡼⠤⠤⠤⢄⣰⠁⠀⠀⠀
⠀⡰⠁⠊⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⡇⠀⠀⠀⠀⠈⣶⡤⣀⠀⠀⠀⠀⠀⠀⠀⠁⠠⣲⠖⠤⢠⠞⠉⠀⠀⠀⠀⠀⠀⠀⠁⠢⡀⠀
⢰⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠁⠉⠛⠒⢧⡀⠀⠀⠀⠀⠘⣷⣀⠉⠑⠒⠂⠒⢐⣦⠖⠋⠀⠀⠀⡗⠀⠀⢀⠀⠀⠀⠀⠀⠀⠀⠀⠐⠀
⠠⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢳⠀⠀⠀⠀⠀⠸⣿⣷⣦⣤⣤⣤⣾⠇⠀⠀⠀⠀⡴⠛⠉⠀⠀⠀⠀⠉⠐⠂⠀⠀⠀⠀⢠
⢰⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠠⠄⣀⡀⢀⠞⢄⠀⠀⠀⠀⠀⠘⢾⣿⣻⣿⣿⡟⠀⠀⠀⠀⢸⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢸
⠈⢆⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠓⢎⠀⠈⠢⡀⠀⠀⠀⠀⠈⠛⠿⠿⢛⠁⠀⠀⠀⠀⠈⢆⣀⣀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈
⠀⠈⢢⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⡜⠻⢤⡀⠈⠲⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⠔⢻⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
⠀⠀⠀⠉⠢⢄⡀⠀⠀⠀⠀⢀⡠⠔⠊⠀⠀⠀⠉⠓⠦⣀⣁⠀⠀⠀⠀⠀⢀⣀⠤⠒⠊⠀⠀⠈⠢⡀⠀⠀⠀⠀⠀⠀⠀⠀⢀⠔⠁⠀
⠀⠀⠀⠀⠀⠀⠀⠉⢉⠉⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠉⠉⠉⠉⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠑⠒⠤⠤⠤⠤⠒⠊⠁⠀⠀⠀
