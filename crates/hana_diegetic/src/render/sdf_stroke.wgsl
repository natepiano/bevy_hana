#define_import_path hana_diegetic::sdf_stroke

fn inflate_subpixel_half_size(
    half_size: vec2<f32>,
    pixel_size: vec2<f32>,
) -> vec3<f32> {
    let min_half = pixel_size * 0.5;
    var effective_half_size = half_size;
    var coverage_scale = 1.0;
    if half_size.x < min_half.x && half_size.x > 0.0 {
        coverage_scale *= half_size.x / min_half.x;
        effective_half_size.x = min_half.x;
    }
    if half_size.y < min_half.y && half_size.y > 0.0 {
        coverage_scale *= half_size.y / min_half.y;
        effective_half_size.y = min_half.y;
    }
    return vec3<f32>(effective_half_size, coverage_scale);
}

// Exact screen-space footprint of a distance field: the L2 length of its
// per-pixel change, length(vec2(dpdx(dist), dpdy(dist))). fwidth() returns
// the L1 sum |dpdx| + |dpdy|, up to sqrt(2) larger when the gradient runs
// diagonally in screen space. That surplus widens the corner anti-aliasing
// band and over-covers corners at grazing angles. Axis-aligned edges, where
// one derivative is near zero, are unchanged.
fn distance_field_band(dist: f32) -> f32 {
    return length(vec2<f32>(dpdx(dist), dpdy(dist)));
}

fn centered_stroke_alpha(outer_dist: f32, inner_dist: f32) -> f32 {
    let stroke_center = 0.5 * (outer_dist + inner_dist);
    let stroke_half_width = max(0.5 * (inner_dist - outer_dist), 0.0);
    let stroke_aa = max(distance_field_band(stroke_center), 0.0001);
    let stroke_shape = 1.0 - smoothstep(
        stroke_half_width - stroke_aa,
        stroke_half_width + stroke_aa,
        abs(stroke_center),
    );
    let border_screen = (2.0 * stroke_half_width) / stroke_aa;
    let coverage = smoothstep(0.0, 1.0, border_screen);
    return stroke_shape * coverage;
}

fn stable_border_alpha(outer_alpha: f32, outer_dist: f32, inner_dist: f32) -> f32 {
    let classic_border_alpha = outer_alpha * (1.0 - (
        1.0 - smoothstep(-max(fwidth(inner_dist) * 0.75, 0.0001), max(fwidth(inner_dist) * 0.75, 0.0001), inner_dist)
    ));
    let thin_stroke_alpha = centered_stroke_alpha(outer_dist, inner_dist);
    let stroke_center = 0.5 * (outer_dist + inner_dist);
    let stroke_half_width = max(0.5 * (inner_dist - outer_dist), 0.0);
    let stroke_aa = max(fwidth(stroke_center), 0.0001);
    let thin_border_mix = 1.0 - smoothstep(0.75, 1.5, stroke_half_width / stroke_aa);
    return mix(classic_border_alpha, thin_stroke_alpha, thin_border_mix);
}

fn stable_line_alpha(center_dist: f32, half_width: f32) -> f32 {
    let stroke_aa = max(fwidth(center_dist), 0.0001);
    let effective_half_width = max(half_width, 0.5 * stroke_aa);
    let coverage = min(half_width / effective_half_width, 1.0);
    let stroke_shape = 1.0 - smoothstep(
        effective_half_width - stroke_aa,
        effective_half_width + stroke_aa,
        center_dist,
    );
    return stroke_shape * coverage;
}

fn rect_strip_alpha(
    local: vec2<f32>,
    half_size: vec2<f32>,
    coverage_scale: f32,
) -> f32 {
    let aa_x = max(fwidth(local.x), 0.0001);
    let aa_y = max(fwidth(local.y), 0.0001);
    let x_alpha = 1.0 - smoothstep(half_size.x - aa_x, half_size.x + aa_x, abs(local.x));
    let y_alpha = 1.0 - smoothstep(half_size.y - aa_y, half_size.y + aa_y, abs(local.y));
    return x_alpha * y_alpha * coverage_scale;
}
