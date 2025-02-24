// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::f32::NAN;
use crate::decoders::*;
use crate::decoders::tiff::*;

/// Decoder for CR3 files - focused on basic EXIF extraction
#[derive(Debug, Clone)]
pub struct Cr3Decoder<'a> {
  buffer: &'a [u8],
  rawloader: &'a RawLoader,
  tiff: TiffIFD<'a>,
}

impl<'a> Cr3Decoder<'a> {
  pub fn new(buf: &'a [u8], tiff: TiffIFD<'a>, rawloader: &'a RawLoader) -> Cr3Decoder<'a> {
    Cr3Decoder {
      buffer: buf,
      tiff: tiff,
      rawloader: rawloader,
    }
  }

  /// Extract basic metadata (make, model) into the camera fields
  fn extract_basic_metadata(&self, camera: &mut Camera) -> Result<(), String> {
    // Extract Make
    if let Some(make) = self.tiff.find_entry(Tag::Make) {
      camera.make = make.get_str().to_string();
      camera.clean_make = make.get_str().to_string();
    }

    // Extract Model
    if let Some(model) = self.tiff.find_entry(Tag::Model) {
      camera.model = model.get_str().to_string();
      camera.clean_model = model.get_str().to_string();
    }

    // Try to find date in EXIF IFD
    if let Some(exif_ifd) = self.tiff.find_first_ifd(Tag::ExifIFDPointer) {
      // Store all EXIF entries in hints for analysis
      if let Some(entry) = exif_ifd.find_entry(Tag::Makernote) {
        camera.hints.push(format!("EXIF: {}", entry.get_str()));
      }
    }

    Ok(())
  }
}

impl<'a> Decoder for Cr3Decoder<'a> {
  fn image(&self, dummy: bool) -> Result<RawImage, String> {
    let mut camera = self.rawloader.check_supported(&self.tiff)?;
    
    // Extract basic metadata into camera fields
    self.extract_basic_metadata(&mut camera)?;
    
    // Get basic image dimensions
    let width = fetch_tag!(self.tiff, Tag::ImageWidth).get_usize(0);
    let height = fetch_tag!(self.tiff, Tag::ImageLength).get_usize(0);

    // Just return a blank image since we only care about metadata
    let image = alloc_image_plain!(width, height, dummy);
    ok_image(camera, width, height, [1.0, 1.0, 1.0, NAN], image)
  }
}