//! Cover art: the picture embedded in a file, an image in the folder, and resizing.
//!
//! Files and folders are only read.

use std::fs::{self, File};
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use lofty::file::TaggedFileExt;
use lofty::picture::{Picture, PictureType};
use lofty::probe::Probe;

/// Names of a folder image, without extension, in order of preference.
const FOLDER_NAMES: [&str; 4] = ["cover", "folder", "front", "album"];

/// Extensions of a folder image.
const FOLDER_EXTENSIONS: [&str; 4] = ["jpg", "jpeg", "png", "webp"];

/// JPEG quality of a resized cover.
const JPEG_QUALITY: u8 = 85;

/// Why cover art could not be read.
#[derive(Debug, thiserror::Error)]
pub enum CoverError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot parse {}: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: lofty::error::FileParseError,
    },
    #[error("cannot process the image: {0}")]
    Image(#[from] image::ImageError),
}

/// An embedded picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub bytes: Vec<u8>,
    /// For example `image/jpeg`.
    pub mime: String,
}

/// The front cover embedded in the file at `path`, else its first picture, if any.
pub fn embedded(path: &Path) -> Result<Option<Image>, CoverError> {
    let file = File::open(path).map_err(|source| CoverError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let tagged = Probe::new(BufReader::new(file))
        .guess_file_type()
        .map_err(|source| CoverError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .read()
        .map_err(|source| CoverError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    let pictures: Vec<&Picture> = tagged.tags().iter().flat_map(|t| t.pictures()).collect();
    let chosen = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first());
    Ok(chosen.map(|p| Image {
        bytes: p.data().to_vec(),
        mime: p
            .mime_type()
            .map_or_else(|| "application/octet-stream".to_owned(), |m| m.to_string()),
    }))
}

/// The folder image of `dir`: the first existing of `cover`, `folder`, `front`, `album`
/// with the extension `jpg`, `jpeg`, `png` or `webp`, ignoring case.
pub fn folder(dir: &Path) -> Option<PathBuf> {
    let names: Vec<(String, PathBuf)> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| Some((e.file_name().to_str()?.to_ascii_lowercase(), e.path())))
        .collect();
    FOLDER_NAMES.iter().find_map(|stem| {
        FOLDER_EXTENSIONS.iter().find_map(|ext| {
            let wanted = format!("{stem}.{ext}");
            names
                .iter()
                .find(|(name, _)| *name == wanted)
                .map(|(_, path)| path.clone())
        })
    })
}

/// `bytes` scaled so that the long side is `size`, as JPEG of quality 85. An image
/// that is smaller keeps its size.
pub fn resize(bytes: &[u8], size: u32) -> Result<Vec<u8>, CoverError> {
    let img = image::load_from_memory(bytes)?;
    let img = if img.width().max(img.height()) > size {
        img.resize(size, size, FilterType::Lanczos3)
    } else {
        img
    };
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(Cursor::new(&mut out), JPEG_QUALITY).encode_image(&rgb)?;
    Ok(out)
}
