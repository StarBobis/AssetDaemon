/*
 * Debug utilities -- provides hex dump and other debug helpers.
 *
 * Follows Soul.md convention: struct + impl organization, no free functions.
 */

/**
 * Debug utility class.
 *
 * Contains hex dump formatting and other general-purpose debug methods.
 * No business logic involved; purely utility methods.
 */
pub struct DebugUtils;

impl DebugUtils {
    /**
     * Format bytes as hex dump, 16 bytes per line with offset.
     *
     * Example output:
     *   00000000: 55 6E 69 74 79 46 53 00  00 00 00 07 35 2E 78 2E  |UnityFS......5.x.|
     *
     * @param data         Byte data to format
     * @param base_offset  Starting offset (displayed in output)
     * @return             hex dump string
     */
    pub fn hex_dump(data: &[u8], base_offset: usize) -> String {
        let mut out = String::new();
        for (i, chunk) in data.chunks(16).enumerate() {
            let offset = base_offset + i * 16;
            out.push_str(&format!("  {:08X}: ", offset));
            for (j, b) in chunk.iter().enumerate() {
                if j == 8 {
                    out.push(' ');
                }
                out.push_str(&format!("{:02X} ", b));
            }
            let pad = 16 - chunk.len();
            for _ in 0..pad {
                out.push_str("   ");
            }
            if pad > 0 && chunk.len() < 8 {
                out.push(' ');
            }
            out.push_str(" |");
            for b in chunk {
                if b.is_ascii_graphic() || *b == b' ' {
                    out.push(*b as char);
                } else {
                    out.push('.');
                }
            }
            out.push_str("|\n");
        }
        out
    }
}
