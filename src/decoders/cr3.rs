// SPDX-License-Identifier: LGPL-2.1
// Copyright 2021 Daniel Vogelbacher <daniel@chaospixel.com>

use std::f32::NAN;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::convert::TryFrom;
use byteorder::{BigEndian, ReadBytesExt, LittleEndian};
use crate::decoders::*;
use crate::decoders::tiff::*;

const BOX_TYPE_CRAW: [u8; 4] = *b"CRAW";
const BOX_TYPE_MOOV: [u8; 4] = *b"moov";
const BOX_TYPE_TRAK: [u8; 4] = *b"trak";
const BOX_TYPE_MDIA: [u8; 4] = *b"mdia";
const BOX_TYPE_MINF: [u8; 4] = *b"minf";
const BOX_TYPE_STBL: [u8; 4] = *b"stbl";
const BOX_TYPE_UUID: [u8; 4] = *b"uuid";
const BOX_TYPE_MDAT: [u8; 4] = *b"mdat";
const BOX_TYPE_CTBO: [u8; 4] = *b"CTBO";

const CR3_COMPATIBLE_BRANDS: [&[u8; 4]; 3] = [
    b"crx ",  // Standard CR3
    b"crx2",  // CR3 version 2
    b"crxm",  // CR3 movie
];

fn is_container_box(box_type: &[u8; 4]) -> bool {
    box_type == &BOX_TYPE_MOOV ||
    box_type == &BOX_TYPE_TRAK ||
    box_type == &BOX_TYPE_MDIA ||
    box_type == &BOX_TYPE_MINF ||
    box_type == &BOX_TYPE_STBL ||
    box_type == &BOX_TYPE_UUID
}

pub fn is_cr3_brand(brand: &[u8; 4]) -> bool {
    CR3_COMPATIBLE_BRANDS.contains(&brand)
}

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

        // Read size and type
        if cursor.read_exact(&mut size_bytes).is_err() {
            return Err("Failed to read box size".to_string());
        }
        if cursor.read_exact(&mut type_bytes).is_err() {
            return Err("Failed to read box type".to_string());
        }

        let mut size = u32::from_be_bytes(size_bytes);
        let mut data_offset = cursor.position();

        // Handle large boxes (size == 1)
        if size == 1 {
            // Skip large boxes for now as they're unlikely to contain CRAW
            return Err("Large boxes not supported yet".to_string());
        }

        // Handle UUID boxes
        if type_bytes == *b"uuid" {
            let mut uuid = [0u8; 16];
            if cursor.read_exact(&mut uuid).is_err() {
                return Err("Failed to read UUID".to_string());
            }
            data_offset += 16;
            eprintln!("Found UUID box: {:02x?}", uuid);
        }

        // Basic size validation
        if size < 8 {
            return Err(format!("Invalid box size {} at offset {}", size, offset));
        }

        // Check if we're still within the file
        if offset + size as u64 > cursor.get_ref().len() as u64 {
            return Err(format!("Box extends beyond file end: size {} at offset {}", size, offset));
        }

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

        let width = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let height = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
        let bit_depth = header[8];
        let components = header[9];
        let component_bit_depth = header[10];

        // Validate header values
        if width == 0 || height == 0 {
            return Err("Invalid CRAW dimensions".to_string());
        }
        if bit_depth == 0 || bit_depth > 32 {
            return Err("Invalid CRAW bit depth".to_string());
        }
        if components == 0 {
            return Err("Invalid CRAW components".to_string());
        }

        eprintln!("CRAW header: {}x{}, {} bit, {} components", width, height, bit_depth, components);

        Ok(CrawHeader {
            width,
            height,
            bit_depth,
            components,
            component_bit_depth,
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

        // Save current position for error reporting
        let data_start = cursor.position();
        eprintln!("Starting raw data decode at offset {}", data_start);

        // For now we're implementing basic raw data reading
        // In a full implementation, we'd need to:
        // 1. Handle different compression methods (JPEG, HEVC)
        // 2. Process color information
        // 3. Handle different bit depths
        let bytes_per_pixel = (header.bit_depth as usize + 7) / 8;
        let row_size = width * bytes_per_pixel;

        for row in 0..height {
            let mut row_data = vec![0u8; row_size];
            if let Err(e) = cursor.read_exact(&mut row_data) {
                return Err(format!("Failed to read raw data at offset {} (row {}): {}",
                    cursor.position(), row, e));
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

        // Parse boxes recursively until we find CRAW
        fn find_craw_box(cursor: &mut Cursor<&[u8]>, decoder: &Cr3Decoder, end_pos: Option<u64>) -> Result<Option<CrawHeader>, String> {
            while let Ok(box_header) = Cr3Decoder::read_box(cursor) {
                let box_end = box_header.offset + box_header.size as u64;
                
                // Check if we've reached the container's end
                if let Some(end) = end_pos {
                    if box_header.offset >= end {
                        break;
                    }
                }
                
                // Convert box type to string for logging
                let box_type = String::from_utf8_lossy(&box_header.box_type);
                eprintln!("Found box: {} at offset {} (size: {}, data offset: {})",
                    box_type, box_header.offset, box_header.size, box_header.data_offset);

                if box_header.box_type == BOX_TYPE_CRAW {
                    eprintln!("Found CRAW box at offset {}", box_header.offset);
                    // Move to the data portion of the CRAW box
                    if let Err(e) = cursor.seek(SeekFrom::Start(box_header.data_offset)) {
                        return Err(format!("Failed to seek to CRAW data: {}", e));
                    }
                    let header = decoder.parse_craw_header(cursor)?;
                    eprintln!("Successfully parsed CRAW header: {}x{} @ {} bit",
                        header.width, header.height, header.bit_depth);
                    return Ok(Some(header));
                }
                
                // Only recurse into container boxes
                if box_header.size > 8 && is_container_box(&box_header.box_type) {
                    eprintln!("Entering container box: {} at offset {}", String::from_utf8_lossy(&box_header.box_type), box_header.offset);
                    
                    // Move to the data portion of the box
                    if let Err(e) = cursor.seek(SeekFrom::Start(box_header.data_offset)) {
                        return Err(format!("Failed to seek in file: {}", e));
                    }
                    
                    // Recursively check this container box
                    if let Some(header) = find_craw_box(cursor, decoder, Some(box_end))? {
                        return Ok(Some(header));
                    }
                    
                    eprintln!("Exiting container box: {} at offset {}", String::from_utf8_lossy(&box_header.box_type), box_header.offset);
                }
                
                // Skip to next box
                if let Err(e) = cursor.seek(SeekFrom::Start(box_end)) {
                    return Err(format!("Failed to seek in file: {}", e));
                }
            }
            Ok(None)
        }

        craw_header = find_craw_box(&mut cursor, self, None)?;

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