const PI: f32 = 3.14159265359;
const DIELECTRIC_F0: vec3<f32> = vec3<f32>(0.04);
const MIN_PERCEPTUAL_ROUGHNESS: f32 = 0.089;

fn pow5(value: f32) -> f32 {
  let squared = value * value;
  return squared * squared * value;
}

fn fresnel_schlick(cosine: f32, f0: vec3<f32>) -> vec3<f32> {
  return f0 + (vec3<f32>(1.0) - f0) * pow5(1.0 - clamp(cosine, 0.0, 1.0));
}

fn fresnel_schlick_roughness(
  no_v: f32,
  f0: vec3<f32>,
  perceptual_roughness: f32,
) -> vec3<f32> {
  let grazing = max(vec3<f32>(1.0 - perceptual_roughness), f0);
  return f0 + (grazing - f0) * pow5(1.0 - clamp(no_v, 0.0, 1.0));
}

fn distribution_ggx(no_h: f32, alpha: f32) -> f32 {
  let alpha_squared = alpha * alpha;
  let denominator = (no_h * alpha_squared - no_h) * no_h + 1.0;
  return alpha_squared / max(PI * denominator * denominator, 0.000001);
}

fn visibility_smith_ggx_correlated_fast(no_v: f32, no_l: f32, alpha: f32) -> f32 {
  let visibility_v = no_l * (no_v * (1.0 - alpha) + alpha);
  let visibility_l = no_v * (no_l * (1.0 - alpha) + alpha);
  return 0.5 / max(visibility_v + visibility_l, 0.0001);
}

fn evaluate_direct_dielectric_f0(
  albedo: vec3<f32>,
  perceptual_roughness: f32,
  f0: vec3<f32>,
  normal: vec3<f32>,
  view_direction: vec3<f32>,
  light_direction: vec3<f32>,
) -> vec3<f32> {
  let no_v = max(dot(normal, view_direction), 0.0001);
  let no_l = max(dot(normal, light_direction), 0.0);
  if no_l <= 0.0 {
    return vec3<f32>(0.0);
  }
  let half_sum = view_direction + light_direction;
  let half_direction = half_sum * inverseSqrt(max(dot(half_sum, half_sum), 0.000001));
  let no_h = max(dot(normal, half_direction), 0.0);
  let lo_h = max(dot(light_direction, half_direction), 0.0);
  let roughness = max(perceptual_roughness, MIN_PERCEPTUAL_ROUGHNESS);
  let alpha = roughness * roughness;
  let fresnel = fresnel_schlick(lo_h, f0);
  let distribution = distribution_ggx(no_h, alpha);
  let visibility = visibility_smith_ggx_correlated_fast(no_v, no_l, alpha);
  let diffuse = albedo * (vec3<f32>(1.0) - fresnel) / PI;
  return (diffuse + distribution * visibility * fresnel) * no_l;
}

fn evaluate_direct_dielectric(
  albedo: vec3<f32>,
  perceptual_roughness: f32,
  normal: vec3<f32>,
  view_direction: vec3<f32>,
  light_direction: vec3<f32>,
) -> vec3<f32> {
  return evaluate_direct_dielectric_f0(
    albedo,
    perceptual_roughness,
    DIELECTRIC_F0,
    normal,
    view_direction,
    light_direction,
  );
}

fn specular_ambient_visibility(no_v: f32, ambient_visibility: f32, roughness: f32) -> f32 {
  // The full grazing-angle correction is only visible on smooth dielectrics. Avoid its two
  // transcendental operations for the rough soil, rock, vegetation, wood, and snow that dominate
  // outdoor pixels; in that range it converges to the ordinary ambient visibility.
  if roughness >= 0.35 {
    return ambient_visibility;
  }
  let exponent = exp2(-16.0 * roughness - 1.0);
  return clamp(
    pow(no_v + ambient_visibility, exponent) - 1.0 + ambient_visibility,
    0.0,
    1.0,
  );
}
