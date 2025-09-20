use std::collections::HashMap;
use std::ops::Range;
use std::sync::mpsc::{channel, Receiver};
#[allow(unused_imports)]
use std::time;
use fastnbt::Value;
use flate2::read::GzDecoder;
use crate::{Error, unwrap_opt_tag, unwrap_tag};
#[allow(unused_imports)]
use crate::world::{AbsolutePosIndexed, Chunk, ChunkPos, ChunkRefAbsolutePos, ChunkVariant, Dimension, FileInfo, FilesInMemory, FilesRead, mca, RefOrObject, XZCoordinate};
use rayon::prelude::*;
use crate::block::Block;
use crate::error::unwrap_opt_i32;
use crate::raid::{Raid, RaidList};
use crate::region::{BlockEntity, HasOffset, PendingTick};
use crate::schem::id_of_nbt_tag;

impl<T> RefOrObject<'_, T> {
    pub fn to_ref(&self) -> &T {
        return match self {
            RefOrObject::Ref(r) => r,
            RefOrObject::Object(o) => &o
        };
    }
}

fn check_chunk_infos(recv: Receiver<(&ChunkPos, Range<i32>)>, num_chunks: usize)
                     -> Result<(), Error> {
    if let Err(_) = recv.try_recv() {
        return Ok(());
    }
    let mut y_range_hist = HashMap::new();
    for (pos, y_range) in recv.try_iter() {
        if !y_range_hist.contains_key(&y_range) {
            y_range_hist.insert(y_range.clone(), Vec::with_capacity(num_chunks));
        }
        let bin = y_range_hist.get_mut(&y_range).unwrap();
        bin.push(*pos);
    }

    if y_range_hist.len() <= 1 {
        return Ok(());
    }
    let mut majority_y_range = 0..0;
    let mut num = 0;

    for (range, bin) in &y_range_hist {
        if bin.len() > num {
            debug_assert!(bin.len() > 0);
            num = bin.len();
            majority_y_range = range.clone();
        }
    }
    if num <= 0 {
        return Ok(());
    }
    let mut exception_value = i32::MAX..i32::MAX;
    let mut exception_chunk = ChunkPos::from_global_pos(&XZCoordinate { x: i32::MAX, z: i32::MAX });
    for (range, bin) in &y_range_hist {
        if range != &majority_y_range {
            exception_value = range.clone();
            exception_chunk = bin[0];
        }
    }
    return Err(Error::DifferentYRangeInOneDimension {
        majority_y_range,
        exception_value,
        exception_chunk_x: exception_chunk.global_x,
        exception_chunk_z: exception_chunk.global_z,
    });
}

fn get_raid_file_name(data_dir: &dyn FilesRead) -> Result<FileInfo, Error> {
    for info in data_dir.files() {
        //println!("filename: {}, full name: {}", info.name, info.full_name);
        if info.name.starts_with("raids") && info.name.ends_with(".dat") {
            return Ok(info);
        }
    }
    return Err(Error::NoSuchFile {
        filename: "raids.dat".to_string(),
        expected_to_exist_in: data_dir.path(),
    });
}

fn parse_raids(data_dir: &dyn FilesRead) -> Result<RaidList, Error> {
    let file = get_raid_file_name(data_dir)?;
    let mut nbt: HashMap<String, Value>;
    {
        let src = data_dir.open_file(&file.name)?;
        let decoder = GzDecoder::new(src);
        match fastnbt::from_reader(decoder) {
            Ok(n) => nbt = n,
            Err(e) => return Err(Error::NBTReadError(e))
        }
    }

    let tag_path_data = format!("{}/data", file.full_name);
    let mut data = unwrap_opt_tag!(nbt.remove("data"),Compound,HashMap::new(),tag_path_data);

    let next_available_id = unwrap_opt_i32(&data, "NextAvailableID", &tag_path_data)?;
    let tick = unwrap_opt_i32(&data, "Tick", &tag_path_data)?;

    let raids_path = format!("{tag_path_data}/Raids");
    let raids_tag = unwrap_opt_tag!(data.remove("Raids"),List,vec![],raids_path);

    let mut raids = Vec::with_capacity(raids_tag.len());
    for (idx, tag) in raids_tag.iter().enumerate() {
        let path = format!("{raids_path}/[{idx}]");
        let tag = unwrap_tag!(tag,Compound,HashMap::new(),path);

        let raid = Raid::from_nbt(tag, &path)?;
        raids.push(raid);
    }

    return Ok(RaidList {
        raids,
        next_available_id,
        tick,
    })
}

impl Dimension {
    pub fn from_files(files: &dyn FilesRead, parse_directly: bool, y_range: Range<i32>, dimension_id: i32) -> Result<Dimension, Error> {
        let chunks = mca::parse_multiple_regions(&files.sub_directory("region"),
                                                 Some(&files.sub_directory("entities")),
                                                 y_range.clone(),
                                                 dimension_id,
                                                 parse_directly)?;

        let raids = parse_raids(&files.sub_directory("data"))?;

        return Ok(Dimension {
            chunks,
            y_range,
            raids,
        });
    }

    pub fn block_pos_to_chunk_pos(block_pos: [i32; 3]) -> (ChunkPos, i8) {
        let cpos = ChunkPos::from_global_pos(&XZCoordinate { x: block_pos[0] / 16, z: block_pos[2] / 16 });
        let y = block_pos[1] / 16;
        return (cpos, y as i8);
    }

    pub fn get_chunk(&self, chunk_pos: &ChunkPos) -> Option<&Chunk> {
        return match self.chunks.get(chunk_pos)? {
            ChunkVariant::Parsed(chunk) => Some(chunk),
            ChunkVariant::Unparsed(_) => None
        };
    }

    pub fn get_chunk_mut(&mut self, chunk_pos: &ChunkPos) -> Option<&mut Chunk> {
        return match self.chunks.get_mut(chunk_pos)? {
            ChunkVariant::Parsed(chunk) => Some(chunk),
            ChunkVariant::Unparsed(_) => None
        };
    }

    pub fn check_all(&self, dimension_id: i32) -> Result<(), Error> {
        let (tx, rx) = channel();

        // Collect chunk infos of all chunks
        let (_chunk_info_tx, chunk_info_rx) = channel();

        self.chunks.par_iter().for_each(|(pos, variant)| {
            match variant.check(pos) {
                Err(e) => { tx.send(e).unwrap(); },
                Ok(chunk) => {
                    if chunk.to_ref().y_range() != self.y_range {
                        tx.send(Error::IncorrectYRangeInChunk {
                            dimension_id,
                            dimension_y_range: self.y_range.clone(),
                            exception_chunk_x: pos.to_global_pos().x,
                            exception_chunk_z: pos.to_global_pos().z,
                            exception_value: chunk.to_ref().y_range(),
                        }).unwrap();
                        return;
                    }

                    //chunk_info_tx.send((pos, chunk.to_ref().y_range())).unwrap()
                }
            }
        });

        if let Ok(e) = rx.try_recv() {
            return Err(e);
        }

        check_chunk_infos(chunk_info_rx, self.chunks.len())?;

        return Ok(());
    }

    pub fn parse_all(&mut self, dimension_id: i32) -> Result<(), Error> {
        let (tx, rx) = channel();
        let (_chunk_info_tx, chunk_info_rx) = channel();
        let num_chunks = self.chunks.len();

        self.chunks.par_iter_mut().for_each(|(pos, variant)| {
            match variant.parse_inplace(pos) {
                Err(e) => tx.send(e).unwrap(),
                Ok(chunk) => {
                    if chunk.y_range() != self.y_range {
                        tx.send(Error::IncorrectYRangeInChunk {
                            dimension_id,
                            dimension_y_range: self.y_range.clone(),
                            exception_chunk_x: pos.to_global_pos().x,
                            exception_chunk_z: pos.to_global_pos().z,
                            exception_value: chunk.y_range(),
                        }).unwrap();
                        return;
                    }

                    //chunk_info_tx.send((pos, chunk.y_range())).unwrap();
                }
            }
        });

        if let Ok(e) = rx.try_recv() {
            return Err(e);
        }

        check_chunk_infos(chunk_info_rx, num_chunks)?;

        return Ok(());
    }
}

impl HasOffset for Dimension {
    fn offset(&self) -> [i32; 3] {
        return [0, 0, 0];
    }
}

// For dimension, since all chunks are stored in the dimension, 'this equals to 'dim
impl<'dim> AbsolutePosIndexed<'dim, 'dim> for Dimension {

    fn shape(&self) -> [i32; 3] {
        let range = self.pos_range();
        return [range[0].len() as i32,
            range[1].len() as i32,
            range[2].len() as i32];
    }

    fn pos_range(&self) -> [Range<i32>; 3] {
        let mut xmin = i32::MAX;
        let mut xmax = i32::MIN;
        let mut zmin = i32::MAX;
        let mut zmax = i32::MIN;
        for (pos, _) in &self.chunks {
            let lb = pos.block_pos_lower_bound();
            let ub = pos.block_pos_upper_bound();
            xmin = xmin.min(lb[0]);
            xmax = xmax.max(ub[0]);
            zmin = zmin.min(lb[1]);
            zmax = zmax.max(ub[1]);
        }
        return [xmin..xmax, self.y_range.clone(), zmin..zmax];
    }

    fn contains_coord(&self, a_pos: [i32; 3]) -> bool {
        let (chunk_pos, _) = Self::block_pos_to_chunk_pos(a_pos);
        if !self.y_range.contains(&a_pos[1]) {
            return false;
        }
        return self.get_chunk(&chunk_pos).is_some();
    }

    fn total_blocks(&self, include_air: bool) -> u64 {
        let mut num_blocks = 0;
        for (_, chunk) in &self.chunks {
            if let ChunkVariant::Parsed(chunk) = chunk {
                num_blocks += chunk.total_blocks(include_air);
            }
        }
        return num_blocks;
    }

    fn block_index_at(&self, a_pos: [i32; 3]) -> Option<u16> {
        if self.contains_coord(a_pos) {
            let (chunk_pos, _y) = Self::block_pos_to_chunk_pos(a_pos);
            let chunk = self.get_chunk(&chunk_pos)?;
            return chunk.as_absolute_pos(&chunk_pos).block_index_at(a_pos);
        }
        return None;
    }

    fn block_at(&'dim self, a_pos: [i32; 3]) -> Option<&'dim Block> {
        if self.contains_coord(a_pos) {
            let (chunk_pos, _y) = Self::block_pos_to_chunk_pos(a_pos);
            let abs: ChunkRefAbsolutePos<'dim> = self.get_chunk(&chunk_pos)?.as_absolute_pos(&chunk_pos);
            return abs.block_at(a_pos);
        }
        return None;
    }

    fn block_entity_at(&'dim self, a_pos: [i32; 3]) -> Option<&'dim BlockEntity> {
        if self.contains_coord(a_pos) {
            let (chunk_pos, _y) = Self::block_pos_to_chunk_pos(a_pos);
            let abs: ChunkRefAbsolutePos<'dim> = self.get_chunk(&chunk_pos)?.as_absolute_pos(&chunk_pos);
            return abs.block_entity_at(a_pos);
        }
        return None;
    }

    fn pending_tick_at(&'dim self, a_pos: [i32; 3]) -> &'dim [PendingTick] {
        if self.contains_coord(a_pos) {
            let (chunk_pos, _y) = Self::block_pos_to_chunk_pos(a_pos);
            let abs = if let Some(chunk) = self.get_chunk(&chunk_pos) {
                chunk.as_absolute_pos(&chunk_pos)
            } else {
                return &[];
            };
            return abs.pending_tick_at(a_pos);
        }
        return &[];
    }
}

#[test]
fn test_load_dimension() {
    let begin = time::SystemTime::now();
    let files = FilesInMemory::from_7z_file("test_files/world/00_1.20.2.7z", "").unwrap();
    let decompressed = time::SystemTime::now();

    let mut dim = Dimension::from_files(&files, false, -64..320, 0).unwrap();
    dim.parse_all(0).unwrap();

    let parsed = time::SystemTime::now();

    println!("{} chunks parsed in {} ms.", dim.chunks.len(), parsed.duration_since(begin).unwrap().as_millis());
    println!("Decompression takes {} ms, parsing takes {} ms",
             decompressed.duration_since(begin).unwrap().as_millis(),
             parsed.duration_since(decompressed).unwrap().as_millis());
}

#[test]
fn test_large_overworld() {
    let begin = time::SystemTime::now();
    let files = FilesInMemory::from_7z_file("test_files/world/01_large-world-1.20.2.7z", "").unwrap();
    let decompressed = time::SystemTime::now();

    let mut dim = Dimension::from_files(&files, false, -64..320, 0).unwrap();

    dim.parse_all(0).unwrap();
    //dim.check_all().unwrap();

    let parsed = time::SystemTime::now();

    println!("{} chunks parsed in {} ms.", dim.chunks.len(), parsed.duration_since(begin).unwrap().as_millis());
    println!("Decompression takes {} ms, parsing takes {} ms",
             decompressed.duration_since(begin).unwrap().as_millis(),
             parsed.duration_since(decompressed).unwrap().as_millis());
}

#[test]
fn test_load_dimension_mcc_block_entities() {
    let begin = time::SystemTime::now();
    let files = FilesInMemory::from_7z_file("test_files/world/02_mcc-block-entities.7z", "").unwrap();
    let decompressed = time::SystemTime::now();


    let mut dim = Dimension::from_files(&files, false, -64..320, 0).unwrap();
    dim.parse_all(0).unwrap();


    let parsed = time::SystemTime::now();

    println!("{} chunks parsed in {} ms.", dim.chunks.len(), parsed.duration_since(begin).unwrap().as_millis());
    println!("Decompression takes {} ms, parsing takes {} ms",
             decompressed.duration_since(begin).unwrap().as_millis(),
             parsed.duration_since(decompressed).unwrap().as_millis());
}

#[test]
fn test_raids() {
    let files = FilesInMemory::from_7z_file("test_files/world/03_raids-1.20.2.7z", "").unwrap();
    // overworld
    Dimension::from_files(&files, false, -64..320, 0).unwrap();
    Dimension::from_files(&files.sub_directory("DIM-1"), false, -64..320, -1).unwrap();
    Dimension::from_files(&files.sub_directory("DIM1"), false, -64..320, 1).unwrap();
}