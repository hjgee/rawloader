// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::f32::NAN;
use std::io::{Cursor, Read, Seek, SeekFrom};
use byteorder::{BigEndian, ReadBytesExt, LittleEndian};
use crate::decoders::*;
use crate::decoders::tiff::*;

const BOX_TYPE_CRAW: [u8; 4] = *b"CRAW";
const BOX_TYPE_TRAK: [u8; 4] = *b"trak";
const BOX_TYPE_MDAT: [u8; 4] = *b"mdat";
const BOX_TYPE_CTBO: [u8; 4] = *b"CTBO";

#[derive(Debug)]
struct Box {
    box_type: [u8; 4],
    size: u32,
    offset: u64,
    data_offset: u64,
}

#[derive(Debug)]
struct CrawHeader {
    width: u32,
    height: u32,
    bit_depth: u8,
    components: u8,
    component_bit_depth: u8,
}

/// Decoder for CR3 files
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

    fn read_box(cursor: &mut Cursor<&[u8]>) -> Result<Box, String> {
        let offset = cursor.position();
        let mut size_bytes = [0u8; 4];
        let mut type_bytes = [0u8; 4];

        if cursor.read_exact(&mut size_bytes).is_err() || 
           cursor.read_exact(&mut type_bytes).is_err() {
            return Err("Failed to read box header".to_string());
        }

        let size = u32::from_be_bytes(size_bytes);
        let data_offset = cursor.position();

        Ok(Box {
            box_type: type_bytes,
            size,
            offset,
            data_offset,
        })
    }

    fn parse_craw_header(&self, cursor: &mut Cursor<&[u8]>) -> Result<CrawHeader, String> {
        let mut header = [0u8; 28];
        if cursor.read_exact(&mut header).is_err() {
            return Err("Failed to read CRAW header".to_string());
        }

        Ok(CrawHeader {
            width: u32::from_be_bytes([header[0], header[1], header[2], header[3]]),
            height: u32::from_be_bytes([header[4], header[5], header[6], header[7]]),
            bit_depth: header[8],
            components: header[9],
            component_bit_depth: header[10],
        })
    }

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

    fn decode_raw_image(&self, cursor: &mut Cursor<&[u8]>, header: &CrawHeader, dummy: bool) -> Result<Vec<u16>, String> {
        let width = header.width as usize;
        let height = header.height as usize;
        let mut image = alloc_image_plain!(width, height, dummy);
        if dummy {
            return Ok(image);
        }

        // For now we're implementing basic raw data reading
        // In a full implementation, we'd need to:
        // 1. Handle different compression methods (JPEG, HEVC)
        // 2. Process color information
        // 3. Handle different bit depths
        let bytes_per_pixel = (header.bit_depth as usize + 7) / 8;
        let row_size = width * bytes_per_pixel;

        for row in 0..height {
            let mut row_data = vec![0u8; row_size];
            if cursor.read_exact(&mut row_data).is_err() {
                return Err("Failed to read raw image data".to_string());
            }

            for col in 0..width {
                let pixel_offset = col * bytes_per_pixel;
                let mut pixel_value = 0u16;

                // Read pixel value based on bit depth
                match bytes_per_pixel {
                    1 => pixel_value = row_data[pixel_offset] as u16,
                    2 => pixel_value = u16::from_le_bytes([
                        row_data[pixel_offset],
                        row_data[pixel_offset + 1]
                    ]),
                    _ => return Err("Unsupported bit depth".to_string()),
                }

                image[row * width + col] = pixel_value;
            }
        }

        Ok(image)
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

        let mut cursor = Cursor::new(self.buffer);
        let mut craw_header = None;

        // Parse boxes until we find CRAW
        while let Ok(box_header) = Self::read_box(&mut cursor) {
            if box_header.box_type == BOX_TYPE_CRAW {
                craw_header = Some(self.parse_craw_header(&mut cursor)?);
                break;
            }
            
            // Skip to next box
            if box_header.size > 8 {
                if let Err(e) = cursor.seek(SeekFrom::Start(box_header.offset + box_header.size as u64)) {
                    return Err(format!("Failed to seek in file: {}", e));
                }
            }
        }

        let header = craw_header.ok_or("Could not find CRAW box")?;
        let width = header.width as usize;
        let height = header.height as usize;

        // Decode the raw image data
        let image = self.decode_raw_image(&mut cursor, &header, dummy)?;

        // For now using neutral WB coefficients
        // In a full implementation, these should be extracted from metadata
        ok_image(camera, width, height, [1.0, 1.0, 1.0, NAN], image)
    }
}