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

    pub fn get_brands(&mut self) -> Result<Vec<[u8; 4]>, String> {
        // Reset to start of file
        if self.reader.seek(SeekFrom::Start(0)).is_err() {
            return Err("Failed to seek to start of file".to_string());
        }

        // Read file type box
        let mut size = [0u8; 4];
        let mut box_type = [0u8; 4];
        let mut major_brand = [0u8; 4];
        
        if self.reader.read_exact(&mut size).is_err() ||
           self.reader.read_exact(&mut box_type).is_err() ||
           self.reader.read_exact(&mut major_brand).is_err() {
            return Err("Failed to read ftyp box header".to_string());
        }

        // Check if it's a 'ftyp' box
        if &box_type != b"ftyp" {
            return Err("Not a valid BMFF file (no ftyp box)".to_string());
        }

        let mut brands = vec![major_brand];

        // Skip minor version
        let mut minor_version = [0u8; 4];
        if self.reader.read_exact(&mut minor_version).is_err() {
            return Ok(brands); // Return just major brand if we can't read more
        }

        // Read compatible brands
        let box_size = u32::from_be_bytes(size);
        if box_size < 16 { // Basic sanity check
            return Ok(brands);
        }
        let num_brands = (box_size as usize - 16) / 4; // 16 = size(4) + type(4) + major_brand(4) + minor_version(4)
        if num_brands > 100 { // Sanity check to prevent excessive iterations
            return Ok(brands);
        }
        
        for _ in 0..num_brands {
            let mut brand_buf = [0u8; 4];
            if self.reader.read_exact(&mut brand_buf).is_err() {
                break;
            }
            brands.push(brand_buf);
        }

        Ok(brands)
    }
}