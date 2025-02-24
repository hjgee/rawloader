// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::f32::NAN;
use std::io::{Cursor, Read, Seek, SeekFrom};
use byteorder::{BigEndian, ReadBytesExt};
use crate::decoders::*;
use crate::decoders::tiff::*;

/// Decoder for CR3 files - focused on basic EXIF extraction
#[derive(Debug)]
pub struct Cr3Decoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: Option<TiffIFD<'a>>,
}

impl<'a> Clone for Cr3Decoder<'a> {
  fn clone(&self) -> Self {
    Cr3Decoder {
      buffer: self.buffer,
      rawloader: self.rawloader,
      tiff: self.tiff.clone(),
    }
  }
}

impl<'a> Cr3Decoder<'a> {
  pub fn new(buf: &'a [u8], tiff: Option<TiffIFD<'a>>, _bmff: Option<()>, rawloader: &'a RawLoader) -> Cr3Decoder<'a> {
    Cr3Decoder {
      buffer: buf,
      tiff,
      rawloader,
    }
  }

  /// Extract basic metadata (make, model) into the camera fields
  fn extract_basic_metadata(&self, camera: &mut Camera) -> Result<(), String> {
    if let Some(ref tiff) = self.tiff {
      // Extract Make
      if let Some(make) = tiff.find_entry(Tag::Make) {
        camera.make = make.get_str().to_string();
        camera.clean_make = make.get_str().to_string();
      }

      // Extract Model
      if let Some(model) = tiff.find_entry(Tag::Model) {
        camera.model = model.get_str().to_string();
        camera.clean_model = model.get_str().to_string();
      }

      // Try to find date in EXIF IFD
      if let Some(exif_ifd) = tiff.find_first_ifd(Tag::ExifIFDPointer) {
        // Store all EXIF entries in hints for analysis
        if let Some(entry) = exif_ifd.find_entry(Tag::Makernote) {
          camera.hints.push(format!("EXIF: {}", entry.get_str()));
        }
      }
    }

    Ok(())
  }

  /// Try to get dimensions from CR3 raw data
  fn get_raw_dimensions(&self) -> Option<(usize, usize)> {
    let mut cursor = Cursor::new(self.buffer);
    let mut box_size = [0u8; 4];
    let mut box_type = [0u8; 4];

    while cursor.read_exact(&mut box_size).is_ok() && cursor.read_exact(&mut box_type).is_ok() {
      let size = u32::from_be_bytes(box_size) as usize;
      if &box_type == b"CRAW" {
        // Found raw image data box
        let mut header = [0u8; 8];
        if cursor.read_exact(&mut header).is_ok() {
          let width = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
          let height = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
          return Some((width, height));
        }
      }
      // Skip to next box
      if size > 8 {
        let _ = cursor.seek(SeekFrom::Current((size - 8) as i64));
      }
    }
    None
  }
}

impl<'a> Decoder for Cr3Decoder<'a> {
  fn image(&self, dummy: bool) -> Result<RawImage, String> {
    let mut camera = if let Some(ref tiff) = self.tiff {
      self.rawloader.check_supported(tiff)?
    } else {
      // If we don't have TIFF data, create a basic camera for CR3
      let mut cam = Camera::new();
      cam.make = "Canon".to_string();
      cam.clean_make = "Canon".to_string();
      cam.model = "EOS CR3".to_string();
      cam.clean_model = "EOS CR3".to_string();
      cam
    };
    
    // Extract basic metadata into camera fields
    self.extract_basic_metadata(&mut camera)?;
    
    // Try to get dimensions from TIFF metadata first, then raw data, then fallback to defaults
    let (width, height) = if let Some(ref tiff) = self.tiff {
      let width = tiff.find_entry(Tag::ImageWidth)
        .map(|w| w.get_usize(0))
        .unwrap_or(0);
      let height = tiff.find_entry(Tag::ImageLength)
        .map(|h| h.get_usize(0))
        .unwrap_or(0);
      if width > 0 && height > 0 {
        (width, height)
      } else {
        self.get_raw_dimensions().unwrap_or((6000, 4000))
      }
    } else {
      self.get_raw_dimensions().unwrap_or((6000, 4000))
    };

    // For now return blank image, but TODO: implement actual CR3 raw data decoding
    let image = alloc_image_plain!(width, height, dummy);
    ok_image(camera, width, height, [1.0, 1.0, 1.0, NAN], image)
  }
}