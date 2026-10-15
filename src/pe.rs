//! Just enough of the PE format to list a DLL's resources (bitmaps, text, dialog templates).

pub struct Resource<'a> {
    /// Resource type id (2 bitmap, 5 dialog, ...); 0 for named types.
    pub kind: u32,
    /// Resource id; 0 for named resources.
    pub id: u32,
    pub data: &'a [u8],
}

pub fn resources(d: &[u8]) -> Result<Vec<Resource<'_>>, String> {
    let u16_at = |o: usize| d.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let bad = || "not a PE file".to_string();
    let pe = u32_at(0x3C).ok_or_else(bad)?;
    if d.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err(bad());
    }
    let sections = u16_at(pe + 6).ok_or_else(bad)?;
    let opt = pe + 24;
    let opt_size = u16_at(pe + 20).ok_or_else(bad)?;
    let dirs = opt + if u16_at(opt) == Some(0x20B) { 112 } else { 96 };
    let res_rva = u32_at(dirs + 2 * 8).ok_or_else(bad)?;
    let mut table = Vec::new();
    for i in 0..sections {
        let s = opt + opt_size + i * 40;
        let field = |o: usize| u32_at(s + o).ok_or_else(bad);
        let (vsize, va, raw_size, raw) = (field(8)?, field(12)?, field(16)?, field(20)?);
        table.push((va, vsize.max(raw_size), raw));
    }
    let to_off = |rva: usize| table.iter().find(|(va, size, _)| rva >= *va && rva < va + size).map(|(va, _, raw)| rva - va + raw);
    let root = to_off(res_rva).ok_or("no resource section")?;
    // Directory entries: (name or id, offset); offsets are relative to the resource root.
    let entries = |dir: usize| -> Vec<(usize, usize)> {
        let n = u16_at(root + dir + 12).unwrap_or(0) + u16_at(root + dir + 14).unwrap_or(0);
        (0..n).filter_map(|i| Some((u32_at(root + dir + 16 + i * 8)?, u32_at(root + dir + 20 + i * 8)?))).collect()
    };
    let id_of = |v: usize| if v & 0x8000_0000 != 0 { 0 } else { v as u32 };
    let mut out = Vec::new();
    for (kind, sub) in entries(0) {
        if sub & 0x8000_0000 == 0 {
            continue;
        }
        for (id, sub2) in entries(sub & 0x7FFF_FFFF) {
            let leaves = if sub2 & 0x8000_0000 != 0 { entries(sub2 & 0x7FFF_FFFF) } else { vec![(0, sub2)] };
            for (_, leaf) in leaves {
                let (Some(rva), Some(size)) = (u32_at(root + leaf), u32_at(root + leaf + 4)) else { continue };
                if let Some(data) = to_off(rva).and_then(|o| d.get(o..o + size)) {
                    out.push(Resource { kind: id_of(kind), id: id_of(id), data });
                }
            }
        }
    }
    Ok(out)
}
