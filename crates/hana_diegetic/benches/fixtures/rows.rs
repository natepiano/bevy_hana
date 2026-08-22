pub type StatusRow = (&'static str, &'static str);

pub const ROW_COUNTS: [usize; 4] = [5, 20, 100, 500];

#[must_use = "benchmarks need generated rows as fixture input"]
pub fn generate_status_rows(count: usize) -> Vec<StatusRow> {
    const LABELS: &[&str] = &[
        "fps:",
        "frame ms:",
        "radius:",
        "entities:",
        "triangles:",
        "draw calls:",
        "memory:",
        "cpu:",
        "gpu:",
        "batches:",
        "lights:",
        "shadows:",
        "textures:",
        "meshes:",
        "shaders:",
        "cameras:",
        "viewports:",
        "particles:",
        "bones:",
        "clips:",
    ];
    const VALUES: &[&str] = &[
        "60", "16.7", "0.3", "1024", "128000", "42", "512MB", "23%", "45%", "18", "4", "8", "256",
        "64", "32", "2", "1", "10000", "128", "3",
    ];
    (0..count)
        .map(|i| (LABELS[i % LABELS.len()], VALUES[i % VALUES.len()]))
        .collect()
}
