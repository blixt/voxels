fn pbr_neutral(color_in: vec3<f32>) -> vec3<f32> {
  let minimum = min(color_in.r, min(color_in.g, color_in.b));
  let offset = select(0.04, minimum - minimum * minimum / 0.16, minimum < 0.08);
  let color = color_in - vec3<f32>(offset);
  let peak = max(color.r, max(color.g, color.b));
  if peak < 0.76 {
    return color;
  }
  let new_peak = 1.0 - 0.0576 / (peak - 0.52);
  let desaturation = 1.0 / (0.15 * (peak - new_peak) + 1.0);
  return mix(vec3<f32>(new_peak), color * (new_peak / peak), desaturation);
}

fn linear_to_srgb(linear: vec3<f32>) -> vec3<f32> {
  let low = linear * 12.92;
  let high = 1.055 * pow(max(linear, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
  return select(high, low, linear <= vec3<f32>(0.0031308));
}
