/*
 * The Magic Blade v1 AssetBundle decryption.
 *
 * The game uses the same AssetStudio SR_CB2 / mr0k bundle encryption path:
 * encrypted LZ4 blocks are prefixed with a small mr0k header, decrypted, then
 * consumed as normal UnityFS LZ4/LZ4HC data.
 */

use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use crate::common::bundle_file::bundle_file::BundleFile;
use crate::common::task::task_logger::TaskLogger;
use crate::utils::compression_utils::CompressionUtils;

const TASK_LABEL: &str = "Decrypt The Magic Blade";

const COMPRESSION_TYPE_MASK: u32 = 0x3f;
const STORAGE_COMPRESSION_TYPE_MASK: u16 = 0x3f;
const COMPRESSION_NONE: u32 = 0;
const COMPRESSION_LZ4HC: u16 = 3;
const COMPRESSION_LZ4_MR0K: u32 = 4;
const BLOCKS_INFO_AT_THE_END: u32 = 0x80;
const BLOCK_INFO_NEED_PADDING_AT_START: u32 = 0x200;

const MR0K_MAGIC: [u8; 4] = [0x6D, 0x72, 0x30, 0x6B];
const MR0K_HEADER_SIZE: usize = 0x94;
const MR0K_PREFIX_SIZE: usize = 0x14;
const MR0K_MAX_ENCRYPTED_BLOCK_SIZE: usize = 0x400;

const SHIFT_ROWS_TABLE_INV: [usize; 16] = [
    0x00, 0x0D, 0x0A, 0x07, 0x04, 0x01, 0x0E, 0x0B, 0x08, 0x05, 0x02, 0x0F, 0x0C, 0x09, 0x06, 0x03,
];

const LOOKUP_SBOX_INV: [u8; 256] = [
    0x52, 0x09, 0x6A, 0xD5, 0x30, 0x36, 0xA5, 0x38, 0xBF, 0x40, 0xA3, 0x9E, 0x81, 0xF3, 0xD7, 0xFB,
    0x7C, 0xE3, 0x39, 0x82, 0x9B, 0x2F, 0xFF, 0x87, 0x34, 0x8E, 0x43, 0x44, 0xC4, 0xDE, 0xE9, 0xCB,
    0x54, 0x7B, 0x94, 0x32, 0xA6, 0xC2, 0x23, 0x3D, 0xEE, 0x4C, 0x95, 0x0B, 0x42, 0xFA, 0xC3, 0x4E,
    0x08, 0x2E, 0xA1, 0x66, 0x28, 0xD9, 0x24, 0xB2, 0x76, 0x5B, 0xA2, 0x49, 0x6D, 0x8B, 0xD1, 0x25,
    0x72, 0xF8, 0xF6, 0x64, 0x86, 0x68, 0x98, 0x16, 0xD4, 0xA4, 0x5C, 0xCC, 0x5D, 0x65, 0xB6, 0x92,
    0x6C, 0x70, 0x48, 0x50, 0xFD, 0xED, 0xB9, 0xDA, 0x5E, 0x15, 0x46, 0x57, 0xA7, 0x8D, 0x9D, 0x84,
    0x90, 0xD8, 0xAB, 0x00, 0x8C, 0xBC, 0xD3, 0x0A, 0xF7, 0xE4, 0x58, 0x05, 0xB8, 0xB3, 0x45, 0x06,
    0xD0, 0x2C, 0x1E, 0x8F, 0xCA, 0x3F, 0x0F, 0x02, 0xC1, 0xAF, 0xBD, 0x03, 0x01, 0x13, 0x8A, 0x6B,
    0x3A, 0x91, 0x11, 0x41, 0x4F, 0x67, 0xDC, 0xEA, 0x97, 0xF2, 0xCF, 0xCE, 0xF0, 0xB4, 0xE6, 0x73,
    0x96, 0xAC, 0x74, 0x22, 0xE7, 0xAD, 0x35, 0x85, 0xE2, 0xF9, 0x37, 0xE8, 0x1C, 0x75, 0xDF, 0x6E,
    0x47, 0xF1, 0x1A, 0x71, 0x1D, 0x29, 0xC5, 0x89, 0x6F, 0xB7, 0x62, 0x0E, 0xAA, 0x18, 0xBE, 0x1B,
    0xFC, 0x56, 0x3E, 0x4B, 0xC6, 0xD2, 0x79, 0x20, 0x9A, 0xDB, 0xC0, 0xFE, 0x78, 0xCD, 0x5A, 0xF4,
    0x1F, 0xDD, 0xA8, 0x33, 0x88, 0x07, 0xC7, 0x31, 0xB1, 0x12, 0x10, 0x59, 0x27, 0x80, 0xEC, 0x5F,
    0x60, 0x51, 0x7F, 0xA9, 0x19, 0xB5, 0x4A, 0x0D, 0x2D, 0xE5, 0x7A, 0x9F, 0x93, 0xC9, 0x9C, 0xEF,
    0xA0, 0xE0, 0x3B, 0x4D, 0xAE, 0x2A, 0xF5, 0xB0, 0xC8, 0xEB, 0xBB, 0x3C, 0x83, 0x53, 0x99, 0x61,
    0x17, 0x2B, 0x04, 0x7E, 0xBA, 0x77, 0xD6, 0x26, 0xE1, 0x69, 0x14, 0x63, 0x55, 0x21, 0x0C, 0x7D,
];

const MR0K_INIT_VECTOR: [u8; 16] = [
    0xA1, 0xF2, 0x7E, 0xB3, 0x5E, 0xDC, 0x88, 0xC7, 0xB6, 0x6C, 0xD8, 0x76, 0xD6, 0x7B, 0xB2, 0x69,
];

const MR0K_EXPANSION_KEY: [u8; 176] = [
    0x2F, 0xE0, 0x89, 0x14, 0xE8, 0x23, 0x4E, 0xE6, 0x91, 0x6D, 0xED, 0xED, 0x86, 0x62, 0x85, 0x1C,
    0xD7, 0x0F, 0x87, 0x2B, 0x64, 0xF3, 0xE0, 0x40, 0xDC, 0x27, 0x17, 0x4C, 0xC2, 0x73, 0x4E, 0x6D,
    0x00, 0xE7, 0x91, 0x04, 0x4E, 0x14, 0x60, 0x8D, 0x1A, 0xE8, 0x36, 0x4F, 0xCF, 0xFB, 0x50, 0xA7,
    0x5D, 0x89, 0x51, 0x7B, 0xBB, 0xEB, 0x4F, 0x1E, 0x56, 0xA0, 0xB0, 0x1F, 0x48, 0x67, 0x86, 0x21,
    0x31, 0xF8, 0xA2, 0xB5, 0xED, 0x90, 0xA4, 0x46, 0xF1, 0x4E, 0x36, 0x38, 0x63, 0x03, 0xA9, 0x8C,
    0x05, 0xD3, 0x2B, 0x17, 0xB2, 0x18, 0x93, 0x34, 0xE0, 0xAA, 0x63, 0x2E, 0x39, 0x99, 0x7D, 0x08,
    0x33, 0x0C, 0x3F, 0xB0, 0x07, 0xEE, 0xD2, 0xB6, 0x8E, 0x0A, 0xB4, 0x3D, 0xA6, 0xBF, 0x77, 0xAF,
    0xBE, 0x17, 0x83, 0xC0, 0x83, 0x2A, 0x57, 0x83, 0xB2, 0xA8, 0xF7, 0xA3, 0xDE, 0xE6, 0x58, 0x4C,
    0xA5, 0x33, 0xF1, 0xE0, 0x9B, 0x3D, 0x7B, 0xC7, 0x26, 0x0B, 0x0E, 0x87, 0xAA, 0x10, 0xB8, 0x5B,
    0xCD, 0x9D, 0x01, 0x0B, 0x64, 0x92, 0xA8, 0xE4, 0xDE, 0x76, 0xA8, 0xE3, 0xAD, 0xA2, 0xC6, 0xB9,
    0x79, 0xBC, 0xEC, 0xF7, 0x37, 0xCF, 0x6C, 0x3D, 0x0D, 0x68, 0xB1, 0x1E, 0xFC, 0x38, 0x43, 0x85,
];

const MR0K_BLOCK_KEY: &[u8] = &[
    0xA1, 0x55, 0x38, 0x57, 0xD0, 0xFA, 0x09, 0xEC, 0xB6, 0x74, 0x76, 0xC7, 0x60, 0x0A, 0xF1, 0x6C,
    0x4C, 0x45, 0xDC, 0x03, 0x80, 0x18, 0x94, 0xDB, 0x75, 0x2E, 0x35, 0xBF, 0x74, 0x29, 0xAE, 0x2D,
    0xFA, 0x37, 0x1A, 0x83, 0x05, 0xAB, 0x2A, 0x47, 0xB8, 0x4D, 0x88, 0xBF, 0xD3, 0x31, 0x96, 0x53,
    0xCB, 0xA9, 0x81, 0xD4, 0xAF, 0x3D, 0xE3, 0xBA, 0xC7, 0x84, 0x23, 0xD2, 0xEC, 0xB2, 0xD7, 0x02,
    0x4F, 0xC2, 0x45, 0xF4, 0x53, 0x56, 0x6E, 0xD6, 0xA6, 0xE8, 0x8C, 0x1D, 0xAE, 0x22, 0x0B, 0x75,
    0xE6, 0xE7, 0x82, 0x88, 0x06, 0x61, 0xB8, 0x4B, 0xA3, 0xCE, 0x52, 0x02, 0x5C, 0x20, 0xE0, 0x19,
    0x03, 0x08, 0xD2, 0x4B, 0x84, 0xF4, 0x11, 0x41, 0xC9, 0xF7, 0xE0, 0x0D, 0x86, 0x09, 0x45, 0xF0,
    0xF2, 0x31, 0xE2, 0x88, 0xC6, 0xE3, 0x3E, 0x40, 0x17, 0x3A, 0xC0, 0x0C, 0x81, 0x8A, 0x80, 0xC3,
    0x76, 0xA4, 0x85, 0x2D, 0x55, 0xB9, 0x6D, 0x32, 0x31, 0xA3, 0xBB, 0xEC, 0xB4, 0x50, 0xA5, 0x7E,
    0xA4, 0x87, 0xEA, 0x5B, 0x9A, 0x67, 0x6C, 0xE0, 0x53, 0x4D, 0x58, 0x29, 0x84, 0x99, 0x83, 0xB0,
    0x04, 0x0A, 0x43, 0x9C, 0xF6, 0xB8, 0x57, 0x5C, 0x68, 0x34, 0x60, 0x36, 0xE6, 0x73, 0x8C, 0xA8,
    0x00, 0xA7, 0x64, 0x91, 0xBE, 0xBE, 0x6F, 0x1E, 0x7B, 0x57, 0x41, 0xB8, 0xAF, 0x13, 0x29, 0x87,
    0x27, 0xE1, 0x64, 0xB5, 0x46, 0xA6, 0xFA, 0x9E, 0x71, 0x15, 0x85, 0xB5, 0x0D, 0x00, 0x0D, 0xA9,
    0x41, 0xFE, 0x1F, 0x05, 0xD8, 0xF2, 0xFA, 0x74, 0x9F, 0x3D, 0xE8, 0x61, 0xA5, 0x06, 0x0A, 0x8A,
    0xC4, 0xF6, 0x64, 0x4F, 0xCE, 0x25, 0x72, 0x81, 0x34, 0xFA, 0x81, 0x0C, 0xC9, 0xC6, 0xF4, 0xA1,
    0x28, 0x4F, 0x14, 0xD3, 0x0E, 0xE1, 0xDB, 0x86, 0xE1, 0x1E, 0x3A, 0xA9, 0xB3, 0xF1, 0x17, 0xDE,
    0x6E, 0x9D, 0x5B, 0x08, 0x79, 0xE0, 0x93, 0x02, 0xF0, 0xAD, 0x2B, 0x21, 0x68, 0x7A, 0xC7, 0xFE,
    0xEB, 0x1E, 0xD9, 0x8B, 0x2C, 0x85, 0xCE, 0x69, 0xDB, 0xDE, 0x6D, 0x30, 0x4B, 0x0E, 0x74, 0x9C,
    0xC8, 0x2D, 0x8A, 0xE5, 0x0C, 0x10, 0x6B, 0xF1, 0x11, 0x41, 0x14, 0x69, 0xF9, 0x5F, 0x68, 0xB8,
    0x6A, 0xD9, 0x0E, 0x03, 0x72, 0x9C, 0x5F, 0x35, 0x2E, 0xB0, 0x88, 0x66, 0x17, 0xB3, 0xE6, 0xCB,
    0xF2, 0xF2, 0xC3, 0xE4, 0x80, 0x4F, 0x6D, 0x07, 0x15, 0x85, 0x31, 0x85, 0x78, 0xC2, 0x60, 0x2E,
    0xB5, 0x7A, 0x16, 0xEE, 0x61, 0xEB, 0x9B, 0x33, 0x71, 0xBD, 0x19, 0x4A, 0xBA, 0xA9, 0x72, 0xA1,
    0xEC, 0x32, 0xFF, 0x27, 0x79, 0x6F, 0x33, 0x2B, 0xC7, 0xC9, 0x88, 0x7B, 0x99, 0x6F, 0x34, 0xA2,
    0xD1, 0x25, 0xC6, 0x8D, 0x91, 0xE7, 0xBA, 0x7E, 0xDD, 0x16, 0x7A, 0x3D, 0x39, 0xC3, 0x07, 0x3A,
    0xD5, 0xE7, 0x0D, 0x48, 0xEB, 0x28, 0x46, 0xE0, 0xE8, 0x6E, 0x8F, 0xDF, 0xA4, 0x67, 0x82, 0x8E,
    0x4E, 0x95, 0xE1, 0xA3, 0x27, 0x1F, 0x54, 0x47, 0x9D, 0x97, 0xA6, 0x21, 0x00, 0x2B, 0x84, 0xBF,
    0xB8, 0x3D, 0x39, 0x74, 0x72, 0x22, 0x9B, 0xC2, 0xDB, 0xEE, 0x3A, 0x9C, 0x9B, 0xB2, 0x79, 0x3D,
    0xBE, 0xAC, 0xAA, 0x63, 0x81, 0xC5, 0xC6, 0x22, 0x32, 0x70, 0x51, 0xC5, 0x30, 0xE6, 0x3A, 0x6B,
    0xF0, 0xCF, 0x35, 0x3D, 0xA0, 0x24, 0xA7, 0xC4, 0x15, 0xA1, 0x78, 0x3B, 0xB1, 0xE0, 0xFE, 0x0C,
    0xF5, 0x9B, 0xBD, 0xA1, 0x5B, 0x5F, 0xE8, 0xAF, 0x76, 0xB7, 0x11, 0x75, 0x12, 0xEF, 0x0A, 0xBF,
    0xE9, 0xBF, 0xE2, 0x73, 0xED, 0x4A, 0xE5, 0x23, 0x82, 0xA4, 0xD0, 0x1C, 0x59, 0xCF, 0x8B, 0x24,
    0xAB, 0xD8, 0x43, 0xFF, 0x30, 0x70, 0xFA, 0xB8, 0x38, 0x24, 0x5A, 0x50, 0x54, 0x13, 0xEB, 0x68,
    0xDD, 0x98, 0xCC, 0xCB, 0x36, 0x65, 0x1A, 0x26, 0x8C, 0xB7, 0x7B, 0x3D, 0x5A, 0x75, 0xE2, 0xD3,
    0x7F, 0x42, 0x91, 0xC1, 0xBD, 0x72, 0xFF, 0x7E, 0x18, 0xCC, 0x0D, 0x39, 0xE9, 0x2D, 0x7F, 0x46,
    0x90, 0xF1, 0xBD, 0x0B, 0x09, 0x5D, 0xD0, 0x0D, 0xEF, 0xAD, 0x93, 0x52, 0xEB, 0x9A, 0x4B, 0x8D,
    0x20, 0x27, 0xD8, 0xE1, 0xE6, 0x30, 0xFD, 0xE2, 0x08, 0xF3, 0x91, 0x61, 0x53, 0x55, 0xC8, 0x14,
    0xAB, 0x19, 0x19, 0x4F, 0xF4, 0x05, 0xEA, 0xFE, 0x76, 0x48, 0xBA, 0xD2, 0xE6, 0x8B, 0x7A, 0xA2,
    0x63, 0xE1, 0x3A, 0x10, 0xE4, 0x48, 0xEB, 0xA9, 0x3C, 0x61, 0x1E, 0x0C, 0x3D, 0x0E, 0x89, 0x2E,
    0xCB, 0x83, 0xEC, 0x15, 0x8E, 0x9B, 0x4D, 0x9F, 0xB9, 0x22, 0xA2, 0x63, 0xAA, 0x59, 0x9F, 0x3E,
    0x96, 0xEB, 0x4B, 0x4F, 0x71, 0x56, 0x15, 0xF2, 0xED, 0x5E, 0x7E, 0x10, 0x64, 0x66, 0x3C, 0xB8,
    0x90, 0x7A, 0x76, 0x6E, 0x2F, 0x6B, 0x43, 0xAB, 0x49, 0x37, 0xBF, 0x42, 0x93, 0x4C, 0x61, 0x63,
    0xF9, 0x92, 0x48, 0x2D, 0xEF, 0x73, 0x86, 0xCC, 0xAC, 0x44, 0x56, 0xC5, 0x53, 0x32, 0x17, 0x7E,
    0x6F, 0x03, 0x0A, 0x6A, 0x7A, 0x68, 0x32, 0x83, 0xE8, 0xDD, 0x64, 0x96, 0x2C, 0x58, 0xB0, 0x12,
    0xAC, 0x92, 0xCD, 0xFA, 0x86, 0x26, 0x69, 0xE3, 0xDF, 0xD8, 0xE9, 0x5A, 0x5C, 0xEF, 0x0E, 0xBE,
    0x77, 0x22, 0xB5, 0xFE, 0xCA, 0x48, 0x67, 0x44, 0xD8, 0xBE, 0x44, 0xF2, 0x92, 0x9C, 0x60, 0x40,
    0xFD, 0xE6, 0xC7, 0x80, 0x09, 0x6A, 0xCD, 0x16, 0x2E, 0xF7, 0x4B, 0x6A, 0x72, 0xE1, 0x96, 0x9B,
    0xE4, 0x4E, 0x99, 0xD8, 0x7E, 0x37, 0x30, 0x6F, 0xB4, 0x07, 0x63, 0x1A, 0x2F, 0x9F, 0x29, 0xB6,
    0x96, 0x08, 0x3A, 0x4C, 0x88, 0x97, 0x8B, 0x83, 0x9A, 0x0F, 0xF7, 0x0B, 0xCD, 0xF5, 0x69, 0x17,
    0x69, 0x23, 0xC0, 0x50, 0x56, 0xC7, 0xA7, 0x66, 0x85, 0x68, 0x37, 0x32, 0xAE, 0x3A, 0x70, 0xB9,
    0x80, 0xEF, 0x3C, 0x28, 0xF9, 0xFF, 0xC4, 0x17, 0xDA, 0x61, 0xB2, 0x35, 0x5D, 0xBE, 0x87, 0x7C,
    0x0B, 0x9F, 0x9E, 0x8A, 0x26, 0x88, 0xA0, 0xB9, 0x2B, 0x90, 0x5E, 0x69, 0x50, 0xFE, 0x16, 0x78,
    0x96, 0x12, 0xD8, 0xFE, 0x2B, 0xEA, 0xA1, 0xB3, 0x89, 0x20, 0x1F, 0xB2, 0x59, 0x3A, 0x6A, 0x25,
    0x2E, 0xA5, 0xA7, 0x6B, 0x93, 0x5C, 0xC7, 0x91, 0x89, 0xCF, 0x99, 0xEC, 0x5A, 0xAF, 0xCB, 0x8D,
    0xC6, 0x79, 0x75, 0x79, 0x32, 0x8A, 0xE0, 0x9A, 0x04, 0xCB, 0xB0, 0x57, 0xB8, 0x75, 0x81, 0xFB,
    0x65, 0x1B, 0xFC, 0xB2, 0xA0, 0x9B, 0xCE, 0xD7, 0x5D, 0x1D, 0x06, 0xDB, 0x6C, 0x46, 0x55, 0x7C,
    0xBC, 0x45, 0x15, 0x2D, 0xBF, 0xC8, 0x0D, 0xB7, 0x02, 0x33, 0x54, 0x16, 0x14, 0xE4, 0xE3, 0xE1,
    0xDF, 0x86, 0x80, 0x7F, 0x4C, 0xE8, 0x8D, 0xA3, 0x97, 0x99, 0xBB, 0x2E, 0x7A, 0x69, 0x60, 0x12,
    0x58, 0x71, 0xF4, 0x50, 0xD1, 0xB2, 0xB0, 0x2E, 0x63, 0x29, 0x3A, 0x63, 0x57, 0x09, 0x99, 0x1A,
    0x98, 0x39, 0x54, 0x65, 0x94, 0x06, 0xD3, 0xC3, 0x31, 0x99, 0x04, 0xD8, 0xAB, 0x5A, 0x3F, 0xA4,
    0xBB, 0xE2, 0x6E, 0x79, 0x70, 0x4D, 0x7A, 0x87, 0x1D, 0x70, 0x55, 0xB0, 0xA6, 0x65, 0x20, 0x44,
    0x54, 0x8D, 0x14, 0x33, 0x78, 0x4D, 0x24, 0x0A, 0x67, 0xBB, 0xE9, 0x3E, 0xE7, 0xCA, 0x5E, 0x98,
    0x26, 0x49, 0x11, 0x9F, 0xE0, 0x0B, 0xAB, 0x03, 0xD0, 0x0C, 0xD3, 0x38, 0xCA, 0xA0, 0xEF, 0xD9,
    0x59, 0xAA, 0x1F, 0xA0, 0x72, 0x8C, 0xC9, 0xBA, 0x99, 0x9D, 0x6F, 0x6B, 0x42, 0x79, 0x3A, 0x9F,
    0x3B, 0xBB, 0x9D, 0x22, 0x88, 0xD5, 0x01, 0x93, 0x2F, 0xC4, 0x23, 0x16, 0xD0, 0xA6, 0x35, 0xA3,
];

#[derive(Clone, serde::Serialize)]
pub struct DecryptResult {
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub failed_files: Vec<String>,
    pub elapsed_secs: f64,
    pub cancelled: bool,
}

#[derive(Clone)]
struct UnityFsHeader {
    signature: String,
    version: u32,
    unity_version: String,
    unity_revision: String,
    compressed_blocks_info_size: u32,
    uncompressed_blocks_info_size: u32,
    flags: u32,
    header_end: usize,
}

#[derive(Clone)]
struct StorageBlockEntry {
    uncompressed_size: u32,
    compressed_size: u32,
    flags: u16,
}

struct BlocksInfoData {
    hash: Vec<u8>,
    blocks: Vec<StorageBlockEntry>,
    rest: Vec<u8>,
}

pub struct TheMagicBladeDecryptionService;

impl TheMagicBladeDecryptionService {
    pub fn decrypt_all_bundles(
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
        input_path: &str,
        output_path: &str,
        max_concurrency: usize,
    ) -> DecryptResult {
        let start_time = Instant::now();
        let input_dir = Path::new(input_path);
        let output_dir = Path::new(output_path);

        if !input_dir.is_dir() {
            let message = format!(
                "Input directory does not exist or is not readable: {}",
                input_path
            );
            TaskLogger::error(task_id, TASK_LABEL, &message);
            return Self::failed_result(message, start_time.elapsed().as_secs_f64());
        }

        if let Err(e) = fs::create_dir_all(output_dir) {
            let message = format!("Cannot create output directory: {}", e);
            TaskLogger::error(task_id, TASK_LABEL, &message);
            return Self::failed_result(message, start_time.elapsed().as_secs_f64());
        }

        let bundle_files = Self::collect_bundle_files_with_task(input_dir, task_id, cancel_token);
        if cancel_token.load(Ordering::SeqCst) {
            TaskLogger::warn(
                task_id,
                TASK_LABEL,
                &format!(
                    "Scan cancelled after finding {} UnityFS files",
                    bundle_files.len()
                ),
            );
            return DecryptResult {
                total: bundle_files.len(),
                success: 0,
                failed: 0,
                failed_files: vec![],
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                cancelled: true,
            };
        }

        if bundle_files.is_empty() {
            TaskLogger::warn(task_id, TASK_LABEL, "No UnityFS files found");
            return DecryptResult {
                total: 0,
                success: 0,
                failed: 0,
                failed_files: vec![],
                elapsed_secs: start_time.elapsed().as_secs_f64(),
                cancelled: false,
            };
        }

        let total = bundle_files.len();
        let worker_count = max_concurrency.max(1).min(total);
        TaskLogger::info(
            task_id,
            TASK_LABEL,
            &format!("Scan complete, found {} UnityFS files", total),
        );
        TaskLogger::progress(task_id, TASK_LABEL, "Decrypt Files", 0, total, "Ready");

        let next_index = Mutex::new(0usize);
        let completed_count = AtomicUsize::new(0);
        let results: Mutex<Vec<(String, Result<u64, String>)>> =
            Mutex::new(Vec::with_capacity(total));

        std::thread::scope(|scope| {
            for _worker_id in 0..worker_count {
                let next_index = &next_index;
                let completed_count = &completed_count;
                let results = &results;
                let bundle_files = &bundle_files;

                scope.spawn(move || loop {
                    if cancel_token.load(Ordering::SeqCst) {
                        break;
                    }

                    let current_idx = {
                        let mut guard = next_index.lock().unwrap();
                        let idx = *guard;
                        *guard = idx + 1;
                        idx
                    };
                    if current_idx >= total {
                        break;
                    }

                    let file_path = &bundle_files[current_idx];
                    let file_name = file_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let result = Self::decrypt_bundle_file(file_path, output_dir);
                    let completed = completed_count.fetch_add(1, Ordering::SeqCst) + 1;
                    match &result {
                        Ok(0) => {
                            TaskLogger::info(
                                task_id,
                                TASK_LABEL,
                                &format!("Skipped existing {}", file_name),
                            );
                            TaskLogger::progress(
                                task_id,
                                TASK_LABEL,
                                "Decrypt Files",
                                completed,
                                total,
                                &format!("Skipped existing {}", file_name),
                            );
                        }
                        Ok(_) => {
                            TaskLogger::info(
                                task_id,
                                TASK_LABEL,
                                &format!("Decrypted {}", file_name),
                            );
                            TaskLogger::progress(
                                task_id,
                                TASK_LABEL,
                                "Decrypt Files",
                                completed,
                                total,
                                &format!("Decrypted {}", file_name),
                            );
                        }
                        Err(error) => {
                            TaskLogger::warn(
                                task_id,
                                TASK_LABEL,
                                &format!("Failed {}: {}", file_name, error),
                            );
                            TaskLogger::progress(
                                task_id,
                                TASK_LABEL,
                                "Decrypt Files",
                                completed,
                                total,
                                &format!("Failed {}", file_name),
                            );
                        }
                    }

                    let mut guard = results.lock().unwrap();
                    guard.push((file_name, result));
                });
            }
        });

        let guard = results.lock().unwrap();
        let success_count = guard.iter().filter(|(_, r)| r.is_ok()).count();
        let failed_files: Vec<String> = guard
            .iter()
            .filter_map(|(name, r)| r.as_ref().err().map(|_| name.clone()))
            .collect();
        let skipped_count = guard.iter().filter(|(_, r)| matches!(r, Ok(0))).count();

        if cancel_token.load(Ordering::SeqCst) {
            TaskLogger::warn(
                task_id,
                TASK_LABEL,
                &format!(
                    "Decrypt cancelled after {} of {} files ({} skipped)",
                    guard.len(),
                    total,
                    skipped_count
                ),
            );
        }

        DecryptResult {
            total,
            success: success_count,
            failed: failed_files.len(),
            failed_files,
            elapsed_secs: start_time.elapsed().as_secs_f64(),
            cancelled: cancel_token.load(Ordering::SeqCst),
        }
    }

    fn failed_result(message: String, elapsed_secs: f64) -> DecryptResult {
        DecryptResult {
            total: 0,
            success: 0,
            failed: 1,
            failed_files: vec![message],
            elapsed_secs,
            cancelled: false,
        }
    }

    fn collect_bundle_files_with_task(
        dir: &Path,
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
    ) -> Vec<PathBuf> {
        let mut bundle_files = Vec::new();
        TaskLogger::info(
            task_id,
            TASK_LABEL,
            &format!("Scanning input directory: {}", dir.display()),
        );
        Self::collect_bundle_files_recursive_with_task(
            dir,
            &mut bundle_files,
            task_id,
            cancel_token,
        );
        bundle_files
    }

    fn collect_bundle_files_recursive_with_task(
        dir: &Path,
        files: &mut Vec<PathBuf>,
        task_id: &str,
        cancel_token: &Arc<AtomicBool>,
    ) {
        if cancel_token.load(Ordering::SeqCst) {
            return;
        }

        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                TaskLogger::warn(
                    task_id,
                    TASK_LABEL,
                    &format!("Cannot read directory '{}': {}", dir.display(), e),
                );
                return;
            }
        };

        for entry in entries.flatten() {
            if cancel_token.load(Ordering::SeqCst) {
                return;
            }

            let path = entry.path();
            if path.is_dir() {
                Self::collect_bundle_files_recursive_with_task(&path, files, task_id, cancel_token);
            } else if Self::is_unityfs_file(&path) {
                files.push(path.clone());
                if files.len() % 5000 == 0 {
                    TaskLogger::info(
                        task_id,
                        TASK_LABEL,
                        &format!(
                            "Found {} UnityFS files, current: {}",
                            files.len(),
                            path.display()
                        ),
                    );
                }
            }
        }
    }

    fn is_unityfs_file(path: &Path) -> bool {
        let Ok(mut file) = fs::File::open(path) else {
            return false;
        };

        let mut magic = [0u8; 8];
        file.read_exact(&mut magic).is_ok() && magic == *b"UnityFS\0"
    }

    fn decrypt_bundle_file(input_path: &Path, output_dir: &Path) -> Result<u64, String> {
        let file_name = input_path
            .file_name()
            .ok_or_else(|| format!("Cannot get file name: {}", input_path.display()))?;
        let output_path = output_dir.join(file_name);
        if output_path.exists() {
            return Ok(0);
        }

        let raw_data = fs::read(input_path)
            .map_err(|e| format!("Cannot read file '{}': {}", input_path.display(), e))?;
        let decrypted_data = Self::decrypt_asset_bundle(&raw_data)?;
        Self::validate_decrypted_bundle(&decrypted_data)?;

        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "Cannot create output directory '{}': {}",
                    parent.display(),
                    e
                )
            })?;
        }

        let mut writer = io::BufWriter::new(fs::File::create(&output_path).map_err(|e| {
            format!(
                "Cannot create output file '{}': {}",
                output_path.display(),
                e
            )
        })?);
        writer
            .write_all(&decrypted_data)
            .map_err(|e| format!("Failed to write file '{}': {}", output_path.display(), e))?;
        writer
            .flush()
            .map_err(|e| format!("Failed to flush buffer '{}': {}", output_path.display(), e))?;

        Ok(decrypted_data.len() as u64)
    }

    pub fn decrypt_asset_bundle(data: &[u8]) -> Result<Vec<u8>, String> {
        let header = Self::read_unityfs_header(data)?;
        if header.signature != "UnityFS" {
            return Err(format!(
                "Unsupported Bundle format: '{}', only UnityFS is supported",
                header.signature
            ));
        }

        let original_blocks_info_start = if (header.flags & BLOCKS_INFO_AT_THE_END) != 0 {
            data.len()
                .checked_sub(header.compressed_blocks_info_size as usize)
                .ok_or_else(|| {
                    format!(
                        "BlocksInfoAtEnd out of bounds: total_len={}, compressed_blocks_info_size={}",
                        data.len(),
                        header.compressed_blocks_info_size
                    )
                })?
        } else {
            header.header_end
        };
        let original_blocks_info_end =
            original_blocks_info_start + header.compressed_blocks_info_size as usize;
        let original_blocks_info_bytes = Self::slice_checked(
            data,
            original_blocks_info_start,
            header.compressed_blocks_info_size as usize,
        )?
        .to_vec();

        let effective_blocks_info =
            Self::decrypt_blocks_info_bytes(&header, original_blocks_info_bytes)?;
        let blocks_info = Self::parse_blocks_info(&effective_blocks_info)?;

        let mut original_blocks_start = if (header.flags & BLOCKS_INFO_AT_THE_END) != 0 {
            header.header_end
        } else {
            original_blocks_info_end
        };
        if (header.flags & BLOCK_INFO_NEED_PADDING_AT_START) != 0 {
            original_blocks_start = Self::align_to(original_blocks_start, 16);
        }

        let mut cursor = original_blocks_start;
        let mut output_blocks = Vec::with_capacity(blocks_info.blocks.len());
        let mut output_block_entries = Vec::with_capacity(blocks_info.blocks.len());
        for (index, block) in blocks_info.blocks.iter().enumerate() {
            let compressed_size = block.compressed_size as usize;
            let block_bytes = Self::slice_checked(data, cursor, compressed_size)
                .map_err(|e| format!("Failed to read block {}: {}", index, e))?;
            cursor = cursor.saturating_add(compressed_size);

            let compression_type = (block.flags & STORAGE_COMPRESSION_TYPE_MASK) as u32;
            let (output_bytes, output_flags) = if compression_type == COMPRESSION_LZ4_MR0K {
                let decrypted = if Self::is_mr0k(block_bytes) {
                    Self::decrypt_mr0k(block_bytes)?
                } else {
                    block_bytes.to_vec()
                };
                (
                    decrypted,
                    (block.flags & !STORAGE_COMPRESSION_TYPE_MASK) | COMPRESSION_LZ4HC,
                )
            } else {
                (block_bytes.to_vec(), block.flags)
            };

            output_block_entries.push(StorageBlockEntry {
                uncompressed_size: block.uncompressed_size,
                compressed_size: output_bytes
                    .len()
                    .try_into()
                    .map_err(|_| format!("Decrypted block {} is too large", index))?,
                flags: output_flags,
            });
            output_blocks.push(output_bytes);
        }

        let modified_blocks_info = Self::build_blocks_info(&blocks_info, &output_block_entries)?;
        let mut output = Self::build_output_bundle(&header, &modified_blocks_info, &output_blocks)?;
        let total_size: i64 = output
            .len()
            .try_into()
            .map_err(|_| "Output bundle too large".to_string())?;
        let size_offset = Self::header_size_offset(&output)?;
        output[size_offset..size_offset + 8].copy_from_slice(&total_size.to_be_bytes());

        Ok(output)
    }

    fn validate_decrypted_bundle(data: &[u8]) -> Result<(), String> {
        BundleFile::parse_filtered(data, |_| false)
            .map(|_| ())
            .map_err(|e| {
                format!(
                    "Decrypted output is not a readable UnityFS Bundle: {}. \
                     The input may use an unsupported SR_CB2/mr0k variant or may not be fully decrypted.",
                    e
                )
            })
    }

    fn decrypt_blocks_info_bytes(
        header: &UnityFsHeader,
        blocks_info_bytes: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        let compression_type = header.flags & COMPRESSION_TYPE_MASK;
        let (payload, effective_compression_type) = if compression_type == COMPRESSION_LZ4_MR0K {
            let payload = if Self::is_mr0k(&blocks_info_bytes) {
                Self::decrypt_mr0k(&blocks_info_bytes)?
            } else {
                blocks_info_bytes
            };
            (payload, COMPRESSION_LZ4HC as u32)
        } else {
            (blocks_info_bytes, compression_type)
        };

        CompressionUtils::decompress_data(
            &payload,
            header.uncompressed_blocks_info_size as usize,
            effective_compression_type,
        )
        .map_err(|e| format!("Failed to decompress BlocksInfo: {}", e))
    }

    fn build_output_bundle(
        header: &UnityFsHeader,
        blocks_info: &[u8],
        blocks: &[Vec<u8>],
    ) -> Result<Vec<u8>, String> {
        let output_flags =
            (header.flags & !COMPRESSION_TYPE_MASK & !BLOCKS_INFO_AT_THE_END) | COMPRESSION_NONE;
        let mut output = Vec::new();
        output.extend_from_slice(header.signature.as_bytes());
        output.push(0);
        output.extend_from_slice(&header.version.to_be_bytes());
        output.extend_from_slice(header.unity_version.as_bytes());
        output.push(0);
        output.extend_from_slice(header.unity_revision.as_bytes());
        output.push(0);
        output.extend_from_slice(&0i64.to_be_bytes());
        output.extend_from_slice(&(blocks_info.len() as u32).to_be_bytes());
        output.extend_from_slice(&(blocks_info.len() as u32).to_be_bytes());
        output.extend_from_slice(&output_flags.to_be_bytes());

        if header.version >= 7 {
            Self::pad_to_alignment(&mut output, 16);
        }
        output.extend_from_slice(blocks_info);
        if (output_flags & BLOCK_INFO_NEED_PADDING_AT_START) != 0 {
            Self::pad_to_alignment(&mut output, 16);
        }
        for block in blocks {
            output.extend_from_slice(block);
        }
        Ok(output)
    }

    fn parse_blocks_info(data: &[u8]) -> Result<BlocksInfoData, String> {
        let mut offset = 0usize;
        let hash = Self::read_bytes(data, &mut offset, 16)?.to_vec();
        let blocks_count = Self::read_i32_be(data, &mut offset)?;
        if !(0..=200000).contains(&blocks_count) {
            return Err(format!("Abnormal block count: {}", blocks_count));
        }

        let mut blocks = Vec::with_capacity(blocks_count as usize);
        for _ in 0..blocks_count {
            blocks.push(StorageBlockEntry {
                uncompressed_size: Self::read_u32_be(data, &mut offset)?,
                compressed_size: Self::read_u32_be(data, &mut offset)?,
                flags: Self::read_u16_be(data, &mut offset)?,
            });
        }

        let rest = data
            .get(offset..)
            .ok_or_else(|| "BlocksInfo remainder is out of bounds".to_string())?
            .to_vec();
        Ok(BlocksInfoData { hash, blocks, rest })
    }

    fn build_blocks_info(
        original: &BlocksInfoData,
        blocks: &[StorageBlockEntry],
    ) -> Result<Vec<u8>, String> {
        if original.blocks.len() != blocks.len() {
            return Err(format!(
                "Block count changed unexpectedly: original={}, output={}",
                original.blocks.len(),
                blocks.len()
            ));
        }

        let mut output =
            Vec::with_capacity(original.hash.len() + 4 + blocks.len() * 10 + original.rest.len());
        output.extend_from_slice(&original.hash);
        output.extend_from_slice(&(blocks.len() as i32).to_be_bytes());
        for block in blocks {
            output.extend_from_slice(&block.uncompressed_size.to_be_bytes());
            output.extend_from_slice(&block.compressed_size.to_be_bytes());
            output.extend_from_slice(&block.flags.to_be_bytes());
        }
        output.extend_from_slice(&original.rest);
        Ok(output)
    }

    fn read_unityfs_header(data: &[u8]) -> Result<UnityFsHeader, String> {
        let mut offset = 0usize;
        let signature = Self::read_string_to_null_limited(data, &mut offset, 32)?;
        let version = Self::read_u32_be(data, &mut offset)?;
        let unity_version = Self::read_string_to_null_limited(data, &mut offset, 1024)?;
        let unity_revision = Self::read_string_to_null_limited(data, &mut offset, 1024)?;
        let _size = Self::read_i64_be(data, &mut offset)?;
        let compressed_blocks_info_size = Self::read_u32_be(data, &mut offset)?;
        let uncompressed_blocks_info_size = Self::read_u32_be(data, &mut offset)?;
        let flags = Self::read_u32_be(data, &mut offset)?;

        Ok(UnityFsHeader {
            signature,
            version,
            unity_version,
            unity_revision,
            compressed_blocks_info_size,
            uncompressed_blocks_info_size,
            flags,
            header_end: offset,
        })
    }

    fn header_size_offset(data: &[u8]) -> Result<usize, String> {
        let mut offset = 0usize;
        Self::read_string_to_null_limited(data, &mut offset, 32)?;
        let _version = Self::read_u32_be(data, &mut offset)?;
        Self::read_string_to_null_limited(data, &mut offset, 1024)?;
        Self::read_string_to_null_limited(data, &mut offset, 1024)?;
        Ok(offset)
    }

    fn decrypt_mr0k(data: &[u8]) -> Result<Vec<u8>, String> {
        if data.len() < MR0K_HEADER_SIZE {
            return Err(format!(
                "mr0k block is too small: len={}, required={}",
                data.len(),
                MR0K_HEADER_SIZE
            ));
        }

        let mut buffer = data.to_vec();
        let mut key1 = [0u8; 16];
        let mut key2 = [0u8; 16];
        let mut key3 = [0u8; 16];
        key1.copy_from_slice(&buffer[4..0x14]);
        key2.copy_from_slice(&buffer[0x74..0x84]);
        key3.copy_from_slice(&buffer[0x84..0x94]);

        for (byte, init) in key2.iter_mut().zip(MR0K_INIT_VECTOR.iter()) {
            *byte ^= *init;
        }

        Self::aes_decrypt_block(&mut key1, &MR0K_EXPANSION_KEY);
        Self::aes_decrypt_block(&mut key3, &MR0K_EXPANSION_KEY);

        for i in 0..16 {
            key1[i] ^= key3[i];
        }
        buffer[0x84..0x94].copy_from_slice(&key1);

        let seed1 = u64::from_le_bytes(
            key2[0..8]
                .try_into()
                .map_err(|_| "Failed to read mr0k seed1".to_string())?,
        );
        let seed2 = u64::from_le_bytes(
            key3[0..8]
                .try_into()
                .map_err(|_| "Failed to read mr0k seed2".to_string())?,
        );
        let seed = seed2
            ^ seed1
            ^ seed1
                .wrapping_add(data.len() as u32 as u64)
                .wrapping_sub(20);
        let seed_bytes = seed.to_le_bytes();
        let encrypted_block_size =
            (0x10 * ((data.len() - MR0K_HEADER_SIZE) >> 7)).min(MR0K_MAX_ENCRYPTED_BLOCK_SIZE);

        for i in 0..encrypted_block_size {
            buffer[MR0K_HEADER_SIZE + i] ^=
                seed_bytes[i % seed_bytes.len()] ^ MR0K_BLOCK_KEY[i % MR0K_BLOCK_KEY.len()];
        }

        Ok(buffer[MR0K_PREFIX_SIZE..].to_vec())
    }

    fn is_mr0k(data: &[u8]) -> bool {
        data.len() >= MR0K_MAGIC.len() && data[..4] == MR0K_MAGIC
    }

    fn aes_decrypt_block(block: &mut [u8; 16], keys: &[u8; 176]) {
        let mut state = *block;
        Self::xor_round_key(&mut state, keys, 0);
        for round in 0..9 {
            Self::sub_bytes_inv(&mut state);
            Self::shift_rows_inv(&mut state);
            Self::mix_cols_inv(&mut state);
            Self::xor_round_key(&mut state, keys, round + 1);
        }
        Self::sub_bytes_inv(&mut state);
        Self::shift_rows_inv(&mut state);
        Self::xor_round_key(&mut state, keys, 0xA);
        *block = state;
    }

    fn sub_bytes_inv(state: &mut [u8; 16]) {
        for byte in state.iter_mut() {
            *byte = LOOKUP_SBOX_INV[*byte as usize];
        }
    }

    fn shift_rows_inv(state: &mut [u8; 16]) {
        let temp = *state;
        for i in 0..16 {
            state[i] = temp[SHIFT_ROWS_TABLE_INV[i]];
        }
    }

    fn mix_cols_inv(state: &mut [u8; 16]) {
        Self::mix_col_inv(state, 0x00);
        Self::mix_col_inv(state, 0x04);
        Self::mix_col_inv(state, 0x08);
        Self::mix_col_inv(state, 0x0C);
    }

    fn mix_col_inv(state: &mut [u8; 16], off: usize) {
        let a0 = state[off];
        let a1 = state[off + 1];
        let a2 = state[off + 2];
        let a3 = state[off + 3];
        state[off] = Self::gf_mul(a0, 14)
            ^ Self::gf_mul(a3, 9)
            ^ Self::gf_mul(a2, 13)
            ^ Self::gf_mul(a1, 11);
        state[off + 1] = Self::gf_mul(a1, 14)
            ^ Self::gf_mul(a0, 9)
            ^ Self::gf_mul(a3, 13)
            ^ Self::gf_mul(a2, 11);
        state[off + 2] = Self::gf_mul(a2, 14)
            ^ Self::gf_mul(a1, 9)
            ^ Self::gf_mul(a0, 13)
            ^ Self::gf_mul(a3, 11);
        state[off + 3] = Self::gf_mul(a3, 14)
            ^ Self::gf_mul(a2, 9)
            ^ Self::gf_mul(a1, 13)
            ^ Self::gf_mul(a0, 11);
    }

    fn gf_mul(mut a: u8, mut b: u8) -> u8 {
        let mut result = 0u8;
        while b != 0 {
            if (b & 1) != 0 {
                result ^= a;
            }
            let carry = (a & 0x80) != 0;
            a <<= 1;
            if carry {
                a ^= 0x1B;
            }
            b >>= 1;
        }
        result
    }

    fn xor_round_key(state: &mut [u8; 16], keys: &[u8; 176], round: usize) {
        for i in 0..16 {
            state[i] ^= keys[i + round * 16];
        }
    }

    fn read_string_to_null_limited(
        data: &[u8],
        offset: &mut usize,
        max_len: usize,
    ) -> Result<String, String> {
        let start = *offset;
        let mut end = start;
        while end < data.len() && end - start < max_len {
            if data[end] == 0 {
                let value = String::from_utf8_lossy(&data[start..end]).to_string();
                *offset = end + 1;
                return Ok(value);
            }
            end += 1;
        }
        Err(format!(
            "Failed to read null-terminated string at offset {}",
            start
        ))
    }

    fn read_bytes<'a>(
        data: &'a [u8],
        offset: &mut usize,
        count: usize,
    ) -> Result<&'a [u8], String> {
        let bytes = Self::slice_checked(data, *offset, count)?;
        *offset += count;
        Ok(bytes)
    }

    fn read_u16_be(data: &[u8], offset: &mut usize) -> Result<u16, String> {
        let bytes = Self::read_bytes(data, offset, 2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_i32_be(data: &[u8], offset: &mut usize) -> Result<i32, String> {
        let bytes = Self::read_bytes(data, offset, 4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_u32_be(data: &[u8], offset: &mut usize) -> Result<u32, String> {
        let bytes = Self::read_bytes(data, offset, 4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_i64_be(data: &[u8], offset: &mut usize) -> Result<i64, String> {
        let bytes = Self::read_bytes(data, offset, 8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn slice_checked(data: &[u8], offset: usize, count: usize) -> Result<&[u8], String> {
        let end = offset
            .checked_add(count)
            .ok_or_else(|| format!("Slice overflow: offset={}, count={}", offset, count))?;
        data.get(offset..end).ok_or_else(|| {
            format!(
                "Slice out of bounds: offset={}, count={}, len={}",
                offset,
                count,
                data.len()
            )
        })
    }

    fn align_to(value: usize, alignment: usize) -> usize {
        (value + alignment - 1) & !(alignment - 1)
    }

    fn pad_to_alignment(data: &mut Vec<u8>, alignment: usize) {
        let aligned = Self::align_to(data.len(), alignment);
        data.resize(aligned, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TheMagicBladeDecryptionService, MR0K_BLOCK_KEY, MR0K_EXPANSION_KEY, MR0K_INIT_VECTOR,
    };
    use std::{
        fs,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn mr0k_constants_have_expected_lengths() {
        assert_eq!(MR0K_INIT_VECTOR.len(), 0x10);
        assert_eq!(MR0K_EXPANSION_KEY.len(), 0xB0);
        assert_eq!(MR0K_BLOCK_KEY.len(), 0x400);
    }

    #[test]
    fn decrypts_aes_block_with_sr_cb2_expansion_key() {
        let mut block = [
            0x6D, 0x72, 0x30, 0x6B, 0xA1, 0x55, 0x38, 0x57, 0xD0, 0xFA, 0x09, 0xEC, 0xB6, 0x74,
            0x76, 0xC7,
        ];

        TheMagicBladeDecryptionService::aes_decrypt_block(&mut block, &MR0K_EXPANSION_KEY);

        assert_eq!(
            block,
            [
                0x89, 0x94, 0x3E, 0xA9, 0x56, 0xBB, 0x17, 0xA6, 0x7B, 0xE4, 0x05, 0xE0, 0x7B, 0xFE,
                0x9F, 0xBF,
            ]
        );
    }

    #[test]
    fn rejects_non_unityfs_data() {
        let err =
            TheMagicBladeDecryptionService::decrypt_asset_bundle(b"not a bundle").unwrap_err();

        assert!(err.contains("Failed to read null-terminated string"));
    }

    #[test]
    fn collects_unityfs_files_without_requiring_bundle_extension() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "the_magic_blade_collects_unityfs_{}_{}",
            std::process::id(),
            unique
        ));
        fs::create_dir_all(root.join("nested")).unwrap();

        let ab_path = root.join("asset.ab");
        let extensionless_path = root.join("nested").join("pc");
        let json_path = root.join("manifest.json");
        fs::write(&ab_path, b"UnityFS\0rest").unwrap();
        fs::write(&extensionless_path, b"UnityFS\0rest").unwrap();
        fs::write(&json_path, b"{\"not\":\"a bundle\"}").unwrap();

        let cancel_token = Arc::new(AtomicBool::new(false));
        let mut files = TheMagicBladeDecryptionService::collect_bundle_files_with_task(
            &root,
            "test_the_magic_blade_collects_unityfs",
            &cancel_token,
        );
        files.sort();

        assert_eq!(files, vec![ab_path, extensionless_path]);
        assert!(!cancel_token.load(Ordering::SeqCst));

        fs::remove_dir_all(root).unwrap();
    }
}
