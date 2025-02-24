use std::io::{Read, Seek, SeekFrom, Cursor};

#[derive(Debug)]
pub struct Bmff {
    reader: Cursor<Vec<u8>>,
}

impl Bmff {
    pub fn new(data: &[u8]) -> Result<Self, String> {
        Ok(Bmff {
            reader: Cursor::new(data.to_vec())
        })
    }

    pub fn compatible_brand(&mut self, brand: &str) -> bool {
        // Brand must be exactly 4 bytes
        if brand.len() != 4 {
            return false;
        }

        // Reset to start of file
        if self.reader.seek(SeekFrom::Start(0)).is_err() {
            return false;
        }

        // Read file type box
        let mut size = [0u8; 4];
        let mut box_type = [0u8; 4];
        let mut major_brand = [0u8; 4];
        
        if self.reader.read_exact(&mut size).is_err() ||
           self.reader.read_exact(&mut box_type).is_err() ||
           self.reader.read_exact(&mut major_brand).is_err() {
            return false;
        }

        // Check if it's a 'ftyp' box
        if &box_type != b"ftyp" {
            return false;
        }

        // Check major brand
        if &major_brand == brand.as_bytes() {
            return true;
        }

        // Skip minor version
        let mut minor_version = [0u8; 4];
        if self.reader.read_exact(&mut minor_version).is_err() {
            return false;
        }

        // Read compatible brands
        let box_size = u32::from_be_bytes(size);
        if box_size < 16 { // Basic sanity check
            return false;
        }
        let num_brands = (box_size as usize - 16) / 4; // 16 = size(4) + type(4) + major_brand(4) + minor_version(4)
        if num_brands > 100 { // Sanity check to prevent excessive iterations
            return false;
        }
        
        let mut brand_buf = [0u8; 4];
        for _ in 0..num_brands {
            if self.reader.read_exact(&mut brand_buf).is_err() {
                return false;
            }
            if &brand_buf == brand.as_bytes() {
                return true;
            }
        }

        false
    }
}