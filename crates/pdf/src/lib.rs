//! PDF files, and Illustrator files (which hold one), read by our own code into vector pictures
//! (`veymelo_svg::Doc`): a page's paths with their fills, strokes, dashes and gradients, cut to
//! their clipping paths and seen through soft masks, blended and faded as the file says; text as
//! the outlines of its fonts' glyphs (each run named by its text); meshes of shaded triangles;
//! pictures; optional content (an Illustrator file's layers) as named groups, hidden ones left
//! out. It is drawn sharp at any size and made shape layers like an SVG.
//!
//! The file: its cross-reference tables or streams (with their updates, and found again when they
//! are wrong), object streams, the filters (Flate, LZW, ASCII hex and base-85, run lengths, the
//! predictors), files locked for no password (RC4, AES-128 and AES-256). A page is the size of its
//! crop box at 72 to the inch (a point a pixel), turned as the page says. What is not read (some
//! picture codecs, CMaps but Identity and the file's own) is named in the picture's notes.

mod colour;
mod content;
mod crypt;
mod file;
mod filters;
mod function;
mod image;
mod object;
mod page;
mod shading;
mod text;

use veymelo_geom::Mat3;
use veymelo_svg::{Doc, Group};

pub use file::File;

/// Whether bytes look like a PDF file (or an Illustrator file holding one).
pub fn looks_like(bytes: &[u8]) -> bool {
    bytes.get(..1024.min(bytes.len())).is_some_and(|h| h.windows(5).any(|w| w == b"%PDF-"))
}

/// How many pages a PDF file has.
pub fn page_count(bytes: Vec<u8>) -> Result<usize, String> {
    Ok(File::open(bytes)?.pages().len())
}

/// Page `index` (from 0) of a PDF file as a vector picture.
pub fn read(bytes: Vec<u8>, index: usize) -> Result<Doc, String> {
    let file = File::open(bytes)?;
    let pages = file.pages();
    let count = pages.len();
    let page = pages.into_iter().nth(index).ok_or_else(|| if count == 0 { "it has no pages".to_string() } else { format!("it has {count} pages; there is no page {}", index + 1) })?;
    // its content: one stream or several run as one
    let mut data = Vec::new();
    let contents = file.at(&page.dict, "Contents");
    let streams: Vec<object::Obj> = match contents {
        Some(object::Obj::Array(a)) => a.iter().map(|o| file.resolve(o)).collect(),
        Some(o) => vec![o],
        None => Vec::new(),
    };
    let mut notes = Vec::new();
    for s in streams {
        if let object::Obj::Stream(s) = s {
            match file.decoded(&s) {
                Ok(d) => {
                    data.extend(d.data);
                    data.push(b'\n');
                }
                Err(e) => notes.push(format!("page content that cannot be read ({e})")),
            }
        }
    }
    let crop = page.crop;
    let mut runner = page::Runner::new(&file, crop);
    let nodes = runner.page(&data, &page.resources);
    notes.extend(runner.notes);
    notes.sort();
    notes.dedup();
    // the page's space (y up, from the crop box's corner) to the picture's pixels, turned
    let (w, h) = (crop[2] - crop[0], crop[3] - crop[1]);
    let flip = Mat3([[1.0, 0.0, -crop[0]], [0.0, -1.0, crop[3]], [0.0, 0.0, 1.0]]);
    let (turn, width, height) = match page.rotate {
        90 => (Mat3([[0.0, -1.0, h], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]), h, w),
        180 => (Mat3([[-1.0, 0.0, w], [0.0, -1.0, h], [0.0, 0.0, 1.0]]), w, h),
        270 => (Mat3([[0.0, 1.0, 0.0], [-1.0, 0.0, w], [0.0, 0.0, 1.0]]), h, w),
        _ => (Mat3::IDENTITY, w, h),
    };
    Ok(Doc { width, height, root: Group::new("pdf", turn * flip, 1.0, nodes), skipped: notes })
}

#[cfg(test)]
mod tests;
