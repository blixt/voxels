import { buildCameraMatrices, type CameraState } from "./camera.ts";
import type { ChunkMeshData } from "./types.ts";
import type { ResidentChunkWorld } from "./world.ts";
import {
  DEFAULT_SKY_WEATHER_ENVIRONMENT,
  LIGHT_DIRECTION,
  LIGHTING_TERMS,
  type SkyWeatherEnvironment,
} from "./render-constants.ts";
import {
  DEFAULT_RENDER_ENVIRONMENT,
  type RenderEnvironment,
} from "./water-visuals.ts";

type RenderCamera = CameraState | {
  viewProjection: Float32Array;
  position?: readonly [number, number, number];
};

const SHADER_SOURCE = `
struct Uniforms {
  view_projection: mat4x4<f32>,
  light_direction: vec4<f32>,
  lighting_terms: vec4<f32>,
  camera_position: vec4<f32>,
  fog_color: vec4<f32>,
  fog_params: vec4<f32>,
  sky_top_color: vec4<f32>,
  sky_horizon_color: vec4<f32>,
  sky_cloud_color: vec4<f32>,
  sky_params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexInput {
  @location(0) position: vec3<f32>,
  @location(1) normal: vec4<f32>,
  @location(2) color: vec4<f32>,
}

struct VertexOutput {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) normal: vec3<f32>,
  @location(1) color: vec4<f32>,
  @location(2) world_position: vec3<f32>,
}

struct SkyVertexOutput {
  @builtin(position) clip_position: vec4<f32>,
  @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
  var output: VertexOutput;
  output.clip_position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
  output.normal = normalize(input.normal.xyz);
  output.color = input.color;
  output.world_position = input.position;
  return output;
}

fn hash_grain(cell: vec3<f32>) -> f32 {
  return fract(sin(dot(cell, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

fn shade_fragment(input: VertexOutput) -> vec4<f32> {
  let directional = max(dot(input.normal, -uniforms.light_direction.xyz), 0.0);
  let hemi = input.normal.y * 0.5 + 0.5;
  let lighting = uniforms.lighting_terms.x + uniforms.lighting_terms.y * directional + uniforms.lighting_terms.z * hemi;
  let albedo_luma = dot(input.color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
  let broad_grain = hash_grain(floor(input.world_position * 0.18));
  let fine_grain = hash_grain(floor(input.world_position * 0.73 + vec3<f32>(19.0, 7.0, 31.0)));
  let grain = (broad_grain - 0.5) * 0.16 + (fine_grain - 0.5) * 0.06;
  let top_bias = abs(input.normal.y);
  let grain_strength = mix(0.045, 0.11, top_bias);
  let ashfall = clamp(uniforms.sky_params.z, 0.0, 1.0);
  let fungal_glow = clamp(uniforms.sky_params.w, 0.0, 1.0);
  let grounded_base = mix(vec3<f32>(albedo_luma), input.color.rgb, 0.78) * (1.0 + grain * grain_strength);
  let ash_deposit = ashfall * smoothstep(0.12, 0.92, input.normal.y) * (0.16 + broad_grain * 0.24);
  let grounded_albedo = mix(grounded_base, uniforms.sky_cloud_color.rgb, clamp(ash_deposit, 0.0, 0.38));
  let light_tint = mix(
    vec3<f32>(0.82, 0.86, 0.92),
    vec3<f32>(1.08, 1.02, 0.92),
    clamp(directional * 0.78 + hemi * 0.22, 0.0, 1.0),
  );
  let overcast = 1.0 - ashfall * 0.18;
  let fungal_shadow = vec3<f32>(0.04, 0.16, 0.18) * fungal_glow * (1.0 - directional) * (1.0 - hemi * 0.46);
  let shaded = grounded_albedo * lighting * light_tint * overcast + fungal_shadow;
  let fog_distance = distance(input.world_position, uniforms.camera_position.xyz);
  let fog = smoothstep(uniforms.fog_params.x, uniforms.fog_params.y, fog_distance);
  let distance_luma = dot(shaded, vec3<f32>(0.2126, 0.7152, 0.0722));
  let distance_muted = mix(shaded, vec3<f32>(distance_luma), fog * 0.22);
  return vec4<f32>(distance_muted * (1.0 - fog) + uniforms.fog_color.rgb * fog, input.color.a);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  return shade_fragment(input);
}

fn sky_hash(cell: vec2<f32>) -> f32 {
  return fract(sin(dot(cell, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

@vertex
fn vs_sky(@builtin(vertex_index) vertex_index: u32) -> SkyVertexOutput {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(3.0, -1.0),
    vec2<f32>(-1.0, 3.0),
  );
  let position = positions[vertex_index];
  var output: SkyVertexOutput;
  output.clip_position = vec4<f32>(position, 1.0, 1.0);
  output.uv = position * 0.5 + vec2<f32>(0.5);
  return output;
}

@fragment
fn fs_sky(input: SkyVertexOutput) -> @location(0) vec4<f32> {
  let y = clamp(input.uv.y, 0.0, 1.0);
  let horizon_blend = smoothstep(0.05, 0.95, y);
  let ashfall = clamp(uniforms.sky_params.z, 0.0, 1.0);
  let fungal_glow = clamp(uniforms.sky_params.w, 0.0, 1.0);
  let high_storm = mix(uniforms.sky_top_color.rgb, vec3<f32>(0.10, 0.10, 0.11), ashfall * 0.46);
  let horizon_glow = mix(uniforms.sky_horizon_color.rgb, uniforms.sky_cloud_color.rgb, ashfall * 0.18 + fungal_glow * 0.16);
  let base = mix(horizon_glow, high_storm, horizon_blend);

  let band_center = clamp(uniforms.sky_params.y, 0.18, 0.82);
  let lower_band = smoothstep(band_center - 0.28, band_center - 0.04, y);
  let upper_band = 1.0 - smoothstep(band_center + 0.05, band_center + 0.36, y);
  let band_mask = clamp(lower_band * upper_band, 0.0, 1.0);
  let wide_noise = sky_hash(floor(input.uv * vec2<f32>(14.0, 5.0)));
  let streak_noise = sky_hash(floor(input.uv * vec2<f32>(36.0, 9.0) + vec2<f32>(11.0, 3.0)));
  let cloud_texture = 0.48 + wide_noise * 0.34 + streak_noise * 0.22;
  let cloud_strength = clamp(uniforms.sky_params.x * band_mask * cloud_texture * (0.72 + ashfall * 0.34), 0.0, 0.92);

  let ash_shelf = ashfall * smoothstep(0.18, 0.62, y) * (1.0 - smoothstep(0.74, 0.96, y));
  let fungal_horizon = fungal_glow * (1.0 - smoothstep(0.05, 0.48, y));
  let storm_belly = vec3<f32>(0.15, 0.14, 0.13);
  let storm_tint = mix(
    uniforms.sky_cloud_color.rgb,
    storm_belly,
    ash_shelf * 0.54,
  );
  var sky = mix(base, storm_tint, cloud_strength);
  sky = mix(sky, storm_belly, ashfall * smoothstep(0.66, 1.0, y) * 0.34);
  sky += vec3<f32>(0.00, 0.08, 0.09) * fungal_horizon;
  sky = mix(sky, vec3<f32>(dot(sky, vec3<f32>(0.2126, 0.7152, 0.0722))), ashfall * 0.10);
  return vec4<f32>(clamp(sky, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
`;

interface GpuChunkResources {
  vertexBuffer: GPUBuffer;
  indexBuffer: GPUBuffer;
  vertexCapacity: number;
  indexCapacity: number;
  indexCount: number;
  triangleCount: number;
  waterVertexBuffer: GPUBuffer | null;
  waterIndexBuffer: GPUBuffer | null;
  waterVertexCapacity: number;
  waterIndexCapacity: number;
  waterIndexCount: number;
  waterTriangleCount: number;
  syncRevision: number;
}

export interface RenderMeshSource {
  mesh: ChunkMeshData | null;
  gpuDirty: boolean;
}

interface PassStats {
  drawCalls: number;
  triangles: number;
  frustumCulledChunks: number;
  fogCulledChunks: number;
  lodDrawCalls: number;
  lodDrawCallsByLevel: readonly number[];
}

export interface RenderStats {
  drawCalls: number;
  triangles: number;
  syncResourcesMs: number;
  uploadMs: number;
  uploadChunks: number;
  uploadBytes: number;
  encodeMs: number;
  frustumCulledChunks: number;
  fogCulledChunks: number;
  lodDrawCalls: number;
  lodDrawCallsByLevel: readonly number[];
}

interface ReadbackImage {
  width: number;
  height: number;
  pixels: Uint8ClampedArray;
}

export class GpuFrameTimer {
  static readonly resolveStride = 256;
  readonly querySet: GPUQuerySet;
  readonly resolveBuffer: GPUBuffer;
  readonly readBuffer: GPUBuffer;

  constructor(device: GPUDevice, readonly frameCount: number) {
    this.querySet = device.createQuerySet({
      type: "timestamp",
      count: frameCount * 2,
    });
    this.resolveBuffer = device.createBuffer({
      size: frameCount * GpuFrameTimer.resolveStride,
      usage: GPUBufferUsage.QUERY_RESOLVE | GPUBufferUsage.COPY_SRC,
    });
    this.readBuffer = device.createBuffer({
      size: frameCount * 16,
      usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
    });
  }

  async readResults(): Promise<number[]> {
    await this.readBuffer.mapAsync(GPUMapMode.READ);
    const values = new BigUint64Array(this.readBuffer.getMappedRange().slice(0));
    const results = new Array<number>(this.frameCount);
    for (let index = 0; index < this.frameCount; index += 1) {
      const start = Number(values[index * 2] ?? 0n);
      const end = Number(values[index * 2 + 1] ?? 0n);
      results[index] = Math.max(0, end - start) / 1_000_000;
    }
    this.readBuffer.unmap();
    return results;
  }

  destroy(): void {
    this.querySet.destroy();
    this.resolveBuffer.destroy();
    this.readBuffer.destroy();
  }
}

function extractFrustumPlanes(vp: Float32Array): Float32Array {
  const planes = new Float32Array(24); // 6 planes × (nx, ny, nz, d)
  for (let i = 0; i < 6; i++) {
    const sign = (i & 1) === 0 ? 1 : -1;
    const row = i >> 1; // 0=X, 1=Y, 2=Z
    planes[i * 4 + 0] = vp[3]! + sign * vp[row]!;
    planes[i * 4 + 1] = vp[7]! + sign * vp[4 + row]!;
    planes[i * 4 + 2] = vp[11]! + sign * vp[8 + row]!;
    planes[i * 4 + 3] = vp[15]! + sign * vp[12 + row]!;
    const len = Math.hypot(planes[i * 4]!, planes[i * 4 + 1]!, planes[i * 4 + 2]!);
    if (len > 0) {
      planes[i * 4] /= len;
      planes[i * 4 + 1] /= len;
      planes[i * 4 + 2] /= len;
      planes[i * 4 + 3] /= len;
    }
  }
  return planes;
}

function isAabbVisible(
  planes: Float32Array,
  minX: number, minY: number, minZ: number,
  maxX: number, maxY: number, maxZ: number,
): boolean {
  for (let i = 0; i < 6; i++) {
    const nx = planes[i * 4]!;
    const ny = planes[i * 4 + 1]!;
    const nz = planes[i * 4 + 2]!;
    const d = planes[i * 4 + 3]!;
    if ((nx > 0 ? maxX : minX) * nx + (ny > 0 ? maxY : minY) * ny + (nz > 0 ? maxZ : minZ) * nz + d < 0) {
      return false;
    }
  }
  return true;
}

function isAabbBeyondDistance(
  point: readonly [number, number, number],
  maxDistance: number,
  minX: number, minY: number, minZ: number,
  maxX: number, maxY: number, maxZ: number,
): boolean {
  const dx = point[0] < minX ? minX - point[0] : point[0] > maxX ? point[0] - maxX : 0;
  const dy = point[1] < minY ? minY - point[1] : point[1] > maxY ? point[1] - maxY : 0;
  const dz = point[2] < minZ ? minZ - point[2] : point[2] > maxZ ? point[2] - maxZ : 0;
  return dx * dx + dy * dy + dz * dz > maxDistance * maxDistance;
}

export class WebGpuVoxelRenderer {
  private readonly context: GPUCanvasContext;
  private readonly device: GPUDevice;
  private readonly pipeline: GPURenderPipeline;
  private readonly lodPipelines: GPURenderPipeline[];
  private readonly waterPipeline: GPURenderPipeline;
  private readonly skyPipeline: GPURenderPipeline;
  private readonly uniformBuffer: GPUBuffer;
  private readonly uniformBindGroup: GPUBindGroup;
  private readonly resources = new Map<object, GpuChunkResources>();
  private readonly uniformData = new Float32Array(52);
  private depthTexture: GPUTexture | null = null;
  private depthView: GPUTextureView | null = null;
  private resourceSyncRevision = 0;
  private lastViewProjection: Float32Array | null = null;
  private lastCameraPosition: readonly [number, number, number] = [0, 0, 0];
  readonly format: GPUTextureFormat;
  readonly timestampQuerySupported: boolean;

  private constructor(canvas: HTMLCanvasElement, context: GPUCanvasContext, device: GPUDevice, format: GPUTextureFormat) {
    this.context = context;
    this.device = device;
    this.format = format;
    this.timestampQuerySupported = device.features.has("timestamp-query");
    const shaderModule = device.createShaderModule({ code: SHADER_SOURCE });
    const uniformBuffer = device.createBuffer({
      size: 208,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    const bindGroupLayout = device.createBindGroupLayout({
      entries: [
        {
          binding: 0,
          visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: "uniform" },
        },
      ],
    });
    const pipelineLayout = device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout] });
    const waterBlend: GPUBlendState = {
      color: {
        srcFactor: "src-alpha",
        dstFactor: "one-minus-src-alpha",
        operation: "add",
      },
      alpha: {
        srcFactor: "one",
        dstFactor: "one-minus-src-alpha",
        operation: "add",
      },
    };
    this.pipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: shaderModule,
        entryPoint: "vs_main",
        buffers: [
          {
            arrayStride: 20,
            attributes: [
              { shaderLocation: 0, format: "float32x3", offset: 0 },
              { shaderLocation: 1, format: "snorm8x4", offset: 12 },
              { shaderLocation: 2, format: "unorm8x4", offset: 16 },
            ],
          },
        ],
      },
      fragment: {
        module: shaderModule,
        entryPoint: "fs_main",
        targets: [{ format }],
      },
      primitive: {
        topology: "triangle-list",
        cullMode: "back",
        frontFace: "ccw",
      },
      depthStencil: {
        format: "depth24plus",
        depthWriteEnabled: true,
        depthCompare: "less",
      },
    });
    this.skyPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: shaderModule,
        entryPoint: "vs_sky",
      },
      fragment: {
        module: shaderModule,
        entryPoint: "fs_sky",
        targets: [{ format }],
      },
      primitive: {
        topology: "triangle-list",
        cullMode: "none",
      },
      depthStencil: {
        format: "depth24plus",
        depthWriteEnabled: false,
        depthCompare: "always",
      },
    });
    // Per-LOD-level pipelines with increasing depth bias so finer LOD
    // levels always win over coarser ones in overlapping regions.
    // LOD 1 (index 0) has the least bias, LOD 4 (index 3) has the most.
    this.lodPipelines = [];
    for (let level = 1; level <= 4; level++) {
      this.lodPipelines.push(device.createRenderPipeline({
        layout: pipelineLayout,
        vertex: {
          module: shaderModule,
          entryPoint: "vs_main",
          buffers: [
            {
              arrayStride: 20,
              attributes: [
                { shaderLocation: 0, format: "float32x3", offset: 0 },
                { shaderLocation: 1, format: "snorm8x4", offset: 12 },
                { shaderLocation: 2, format: "unorm8x4", offset: 16 },
              ],
            },
          ],
        },
        fragment: {
          module: shaderModule,
          entryPoint: "fs_main",
          targets: [{ format }],
        },
        primitive: {
          topology: "triangle-list",
          cullMode: "back",
          frontFace: "ccw",
        },
        depthStencil: {
          format: "depth24plus",
          depthWriteEnabled: true,
          depthCompare: "less",
          depthBias: level * 8,
          depthBiasSlopeScale: level * 4,
          depthBiasClamp: 0,
        },
      }));
    }
    this.waterPipeline = device.createRenderPipeline({
      layout: pipelineLayout,
      vertex: {
        module: shaderModule,
        entryPoint: "vs_main",
        buffers: [
          {
            arrayStride: 20,
            attributes: [
              { shaderLocation: 0, format: "float32x3", offset: 0 },
              { shaderLocation: 1, format: "snorm8x4", offset: 12 },
              { shaderLocation: 2, format: "unorm8x4", offset: 16 },
            ],
          },
        ],
      },
      fragment: {
        module: shaderModule,
        entryPoint: "fs_main",
        targets: [{ format, blend: waterBlend }],
      },
      primitive: {
        topology: "triangle-list",
        cullMode: "none",
        frontFace: "ccw",
      },
      depthStencil: {
        format: "depth24plus",
        depthWriteEnabled: false,
        depthCompare: "less",
      },
    });
    this.uniformBuffer = uniformBuffer;
    this.uniformBindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: uniformBuffer } },
      ],
    });
    this.configureCanvas(canvas);
  }

  static async create(canvas: HTMLCanvasElement): Promise<WebGpuVoxelRenderer> {
    if (!navigator.gpu) {
      throw new Error("WebGPU is not available in this version of Chrome.");
    }
    const adapter = await navigator.gpu.requestAdapter({
      powerPreference: "high-performance",
    });
    if (!adapter) {
      throw new Error("Chrome did not return a GPU adapter.");
    }

    const requestedFeatures: GPUFeatureName[] = [];
    if (adapter.features.has("timestamp-query")) {
      requestedFeatures.push("timestamp-query");
    }
    const device = await adapter.requestDevice({ requiredFeatures: requestedFeatures });
    const context = canvas.getContext("webgpu");
    if (!context) {
      throw new Error("Unable to acquire a WebGPU canvas context.");
    }
    const format = navigator.gpu.getPreferredCanvasFormat();
    context.configure({
      device,
      format,
      alphaMode: "opaque",
    });
    return new WebGpuVoxelRenderer(canvas, context, device, format);
  }

  configureCanvas(canvas: HTMLCanvasElement): void {
    const pixelRatio = window.devicePixelRatio || 1;
    const width = Math.max(1, Math.floor(canvas.clientWidth * pixelRatio));
    const height = Math.max(1, Math.floor(canvas.clientHeight * pixelRatio));
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    if (!this.depthTexture || this.depthTexture.width !== width || this.depthTexture.height !== height) {
      this.depthTexture?.destroy();
      this.depthTexture = this.device.createTexture({
        size: [canvas.width, canvas.height, 1],
        format: "depth24plus",
        usage: GPUTextureUsage.RENDER_ATTACHMENT,
      });
      this.depthView = this.depthTexture.createView();
    }
  }

  createFrameTimer(frameCount: number): GpuFrameTimer | null {
    if (!this.timestampQuerySupported) {
      return null;
    }
    return new GpuFrameTimer(this.device, frameCount);
  }

  render(
    world: ResidentChunkWorld,
    camera: RenderCamera,
    timer: GpuFrameTimer | null = null,
    frameIndex = 0,
    renderEnvironment: RenderEnvironment = DEFAULT_RENDER_ENVIRONMENT,
  ): RenderStats {
    this.configureCanvas(this.context.canvas as HTMLCanvasElement);
    const syncStartedAt = performance.now();
    const syncStats = this.syncResources(world);
    const syncResourcesMs = performance.now() - syncStartedAt;
    const canvas = this.context.canvas as HTMLCanvasElement;
    this.writeUniforms(camera, canvas.width / canvas.height, renderEnvironment);

    const frustumPlanes = this.lastViewProjection
      ? extractFrustumPlanes(this.lastViewProjection)
      : null;

    const encoder = this.device.createCommandEncoder();
    const skyWeather = resolveSkyWeatherEnvironment(renderEnvironment);
    const passDescriptor: GPURenderPassDescriptor = {
      colorAttachments: [
        {
          view: this.context.getCurrentTexture().createView(),
          loadOp: "clear",
          clearValue: toGpuColor(skyWeather.skyHorizonColorRgba),
          storeOp: "store",
        },
      ],
      depthStencilAttachment: {
        view: this.depthView!,
        depthLoadOp: "clear",
        depthClearValue: 1,
        depthStoreOp: "store",
      },
    };
    if (timer) {
      passDescriptor.timestampWrites = {
        querySet: timer.querySet,
        beginningOfPassWriteIndex: frameIndex * 2,
        endOfPassWriteIndex: frameIndex * 2 + 1,
      };
    }

    const encodeStartedAt = performance.now();
    const stats = this.encodeRenderPass(
      world,
      encoder,
      passDescriptor,
      frustumPlanes,
      this.lastCameraPosition,
      renderEnvironment.fogEndDistance,
    );
    const encodeMs = performance.now() - encodeStartedAt;

    if (timer) {
      const resolveOffset = frameIndex * GpuFrameTimer.resolveStride;
      encoder.resolveQuerySet(timer.querySet, frameIndex * 2, 2, timer.resolveBuffer, resolveOffset);
      encoder.copyBufferToBuffer(timer.resolveBuffer, resolveOffset, timer.readBuffer, frameIndex * 16, 16);
    }

    this.device.queue.submit([encoder.finish()]);
    return {
      ...stats,
      syncResourcesMs,
      uploadMs: syncStats.uploadMs,
      uploadChunks: syncStats.uploadChunks,
      uploadBytes: syncStats.uploadBytes,
      encodeMs,
    };
  }

  async waitForGpuIdle(): Promise<void> {
    await this.device.queue.onSubmittedWorkDone();
  }

  async captureImage(
    world: ResidentChunkWorld,
    camera: RenderCamera,
    width: number,
    height: number,
    renderEnvironment: RenderEnvironment = DEFAULT_RENDER_ENVIRONMENT,
  ): Promise<ReadbackImage> {
    this.syncResources(world);
    this.writeUniforms(camera, width / height, renderEnvironment);

    const frustumPlanes = this.lastViewProjection
      ? extractFrustumPlanes(this.lastViewProjection)
      : null;

    const colorTexture = this.device.createTexture({
      size: [width, height, 1],
      format: this.format,
      usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC,
    });
    const depthTexture = this.device.createTexture({
      size: [width, height, 1],
      format: "depth24plus",
      usage: GPUTextureUsage.RENDER_ATTACHMENT,
    });
    const bytesPerRow = alignTo(width * 4, 256);
    const readback = this.device.createBuffer({
      size: bytesPerRow * height,
      usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ,
    });

    const encoder = this.device.createCommandEncoder();
    const skyWeather = resolveSkyWeatherEnvironment(renderEnvironment);
    this.encodeRenderPass(world, encoder, {
      colorAttachments: [
        {
          view: colorTexture.createView(),
          loadOp: "clear",
          clearValue: toGpuColor(skyWeather.skyHorizonColorRgba),
          storeOp: "store",
        },
      ],
      depthStencilAttachment: {
        view: depthTexture.createView(),
        depthLoadOp: "clear",
        depthClearValue: 1,
        depthStoreOp: "store",
      },
    }, frustumPlanes, this.lastCameraPosition, renderEnvironment.fogEndDistance);
    encoder.copyTextureToBuffer(
      { texture: colorTexture },
      { buffer: readback, bytesPerRow, rowsPerImage: height },
      { width, height, depthOrArrayLayers: 1 },
    );
    this.device.queue.submit([encoder.finish()]);
    await this.device.queue.onSubmittedWorkDone();

    await readback.mapAsync(GPUMapMode.READ);
    const mapped = new Uint8Array(readback.getMappedRange());
    const pixels = new Uint8ClampedArray(width * height * 4);
    for (let row = 0; row < height; row += 1) {
      const sourceOffset = row * bytesPerRow;
      const targetOffset = row * width * 4;
      for (let column = 0; column < width; column += 1) {
        const base = sourceOffset + column * 4;
        const out = targetOffset + column * 4;
        if (this.format === "bgra8unorm") {
          pixels[out + 0] = mapped[base + 2]!;
          pixels[out + 1] = mapped[base + 1]!;
          pixels[out + 2] = mapped[base + 0]!;
          pixels[out + 3] = mapped[base + 3]!;
        } else {
          pixels[out + 0] = mapped[base + 0]!;
          pixels[out + 1] = mapped[base + 1]!;
          pixels[out + 2] = mapped[base + 2]!;
          pixels[out + 3] = mapped[base + 3]!;
        }
      }
    }
    readback.unmap();
    readback.destroy();
    colorTexture.destroy();
    depthTexture.destroy();
    return { width, height, pixels };
  }

  dispose(): void {
    for (const resource of this.resources.values()) {
      resource.vertexBuffer.destroy();
      resource.indexBuffer.destroy();
    }
    this.resources.clear();
    this.depthTexture?.destroy();
    this.uniformBuffer.destroy();
  }

  private encodeRenderPass(
    world: ResidentChunkWorld,
    encoder: GPUCommandEncoder,
    passDescriptor: GPURenderPassDescriptor,
    frustumPlanes: Float32Array | null,
    cameraPosition: readonly [number, number, number],
    fogEndDistance: number,
  ): PassStats {
    const pass = encoder.beginRenderPass(passDescriptor);
    pass.setBindGroup(0, this.uniformBindGroup);
    pass.setPipeline(this.skyPipeline);
    pass.draw(3, 1, 0, 0);

    let drawCalls = 0;
    let triangles = 0;
    let frustumCulledChunks = 0;
    let fogCulledChunks = 0;
    let lodDrawCalls = 0;
    const lodDrawCallsByLevel = [0, 0, 0, 0, 0];
    let currentPipeline: GPURenderPipeline | null = null;

    // Opaque pass: LOD chunks first (depth-biased), then LOD 0 (standard).
    // iterateResidentChunks yields LOD chunks before LOD 0 chunks.
    // LOD pipeline has depthBias so LOD 0 always wins where both exist.
    for (const chunk of world.iterateResidentChunks()) {
      const resource = this.resources.get(chunk);
      if (!resource || resource.indexCount === 0) {
        continue;
      }
      const b = chunk.mesh?.bounds;
      if (b && frustumPlanes && !isAabbVisible(frustumPlanes, b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2])) {
        frustumCulledChunks++;
        continue;
      }
      if (b && isAabbBeyondDistance(cameraPosition, fogEndDistance, b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2])) {
        fogCulledChunks++;
        continue;
      }
      const pipeline = chunk.lodLevel > 0
        ? this.lodPipelines[Math.min(chunk.lodLevel - 1, this.lodPipelines.length - 1)]!
        : this.pipeline;
      if (pipeline !== currentPipeline) {
        pass.setPipeline(pipeline);
        currentPipeline = pipeline;
      }
      pass.setVertexBuffer(0, resource.vertexBuffer);
      pass.setIndexBuffer(resource.indexBuffer, "uint32");
      pass.drawIndexed(resource.indexCount, 1, 0, 0, 0);
      drawCalls += 1;
      triangles += resource.triangleCount;
      if (chunk.lodLevel > 0) {
        lodDrawCalls += 1;
        lodDrawCallsByLevel[Math.min(chunk.lodLevel, lodDrawCallsByLevel.length - 1)] += 1;
      }
    }

    // Water pass.
    pass.setPipeline(this.waterPipeline);
    for (const chunk of world.iterateResidentChunks()) {
      const resource = this.resources.get(chunk);
      if (!resource || resource.waterIndexCount === 0 || !resource.waterVertexBuffer || !resource.waterIndexBuffer) {
        continue;
      }
      const b = chunk.mesh?.bounds;
      if (b && frustumPlanes && !isAabbVisible(frustumPlanes, b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2])) {
        frustumCulledChunks++;
        continue;
      }
      if (b && isAabbBeyondDistance(cameraPosition, fogEndDistance, b.min[0], b.min[1], b.min[2], b.max[0], b.max[1], b.max[2])) {
        fogCulledChunks++;
        continue;
      }
      pass.setVertexBuffer(0, resource.waterVertexBuffer);
      pass.setIndexBuffer(resource.waterIndexBuffer, "uint32");
      pass.drawIndexed(resource.waterIndexCount, 1, 0, 0, 0);
      drawCalls += 1;
      triangles += resource.waterTriangleCount;
      if (chunk.lodLevel > 0) {
        lodDrawCalls += 1;
        lodDrawCallsByLevel[Math.min(chunk.lodLevel, lodDrawCallsByLevel.length - 1)] += 1;
      }
    }

    pass.end();
    return { drawCalls, triangles, frustumCulledChunks, fogCulledChunks, lodDrawCalls, lodDrawCallsByLevel };
  }

  private writeUniforms(camera: RenderCamera, aspect: number, renderEnvironment: RenderEnvironment): void {
    let viewProjection: Float32Array;
    let cameraPosition: readonly [number, number, number];
    if (isCameraWithViewProjection(camera)) {
      viewProjection = camera.viewProjection;
      cameraPosition = camera.position ?? ([0, 0, 0] as const);
    } else {
      const cameraMatrices = buildCameraMatrices(camera, aspect);
      viewProjection = cameraMatrices.viewProjection;
      cameraPosition = cameraMatrices.eye;
    }
    this.lastViewProjection = viewProjection;
    this.lastCameraPosition = cameraPosition;
    const uniformData = this.uniformData;
    uniformData.set(viewProjection, 0);
    uniformData[16] = LIGHT_DIRECTION[0];
    uniformData[17] = LIGHT_DIRECTION[1];
    uniformData[18] = LIGHT_DIRECTION[2];
    uniformData[19] = 0;
    uniformData[20] = LIGHTING_TERMS[0];
    uniformData[21] = LIGHTING_TERMS[1];
    uniformData[22] = LIGHTING_TERMS[2];
    uniformData[23] = 0;
    uniformData[24] = cameraPosition[0];
    uniformData[25] = cameraPosition[1];
    uniformData[26] = cameraPosition[2];
    uniformData[27] = 0;
    uniformData[28] = renderEnvironment.fogColorRgba[0] / 255;
    uniformData[29] = renderEnvironment.fogColorRgba[1] / 255;
    uniformData[30] = renderEnvironment.fogColorRgba[2] / 255;
    uniformData[31] = renderEnvironment.fogColorRgba[3] / 255;
    uniformData[32] = renderEnvironment.fogStartDistance;
    uniformData[33] = renderEnvironment.fogEndDistance;
    uniformData[34] = 0;
    uniformData[35] = 0;
    const skyWeather = resolveSkyWeatherEnvironment(renderEnvironment);
    uniformData[36] = skyWeather.skyTopColorRgba[0] / 255;
    uniformData[37] = skyWeather.skyTopColorRgba[1] / 255;
    uniformData[38] = skyWeather.skyTopColorRgba[2] / 255;
    uniformData[39] = skyWeather.skyTopColorRgba[3] / 255;
    uniformData[40] = skyWeather.skyHorizonColorRgba[0] / 255;
    uniformData[41] = skyWeather.skyHorizonColorRgba[1] / 255;
    uniformData[42] = skyWeather.skyHorizonColorRgba[2] / 255;
    uniformData[43] = skyWeather.skyHorizonColorRgba[3] / 255;
    uniformData[44] = skyWeather.skyCloudColorRgba[0] / 255;
    uniformData[45] = skyWeather.skyCloudColorRgba[1] / 255;
    uniformData[46] = skyWeather.skyCloudColorRgba[2] / 255;
    uniformData[47] = skyWeather.skyCloudColorRgba[3] / 255;
    uniformData[48] = skyWeather.skyCloudCoverage;
    uniformData[49] = skyWeather.skyCloudBand;
    uniformData[50] = skyWeather.ashfallIntensity;
    uniformData[51] = skyWeather.fungalGlowIntensity;
    this.device.queue.writeBuffer(this.uniformBuffer, 0, uniformData);
  }

  private syncResources(world: ResidentChunkWorld): {
    uploadMs: number;
    uploadChunks: number;
    uploadBytes: number;
  } {
    this.resourceSyncRevision += 1;
    const syncRevision = this.resourceSyncRevision;
    let uploadMs = 0;
    let uploadChunks = 0;
    let uploadBytes = 0;

    for (const chunk of world.iterateResidentChunks()) {
      const upload = this.syncMeshSourceResource(chunk, chunk.mesh, syncRevision);
      uploadMs += upload.elapsedMs;
      uploadChunks += upload.updated ? 1 : 0;
      uploadBytes += upload.totalBytes;
    }

    for (const [resourceKey, resource] of this.resources.entries()) {
      if (resource.syncRevision === syncRevision) {
        continue;
      }
      this.destroyResource(resourceKey, resource);
    }
    return { uploadMs, uploadChunks, uploadBytes };
  }

  private syncMeshSourceResource(
    source: RenderMeshSource,
    mesh: ChunkMeshData | null,
    syncRevision: number,
  ): {
    elapsedMs: number;
    totalBytes: number;
    updated: boolean;
  } {
    const existing = this.resources.get(source);
    if (!mesh || (mesh.indexCount === 0 && mesh.waterIndexCount === 0)) {
      if (existing) {
        this.destroyResource(source, existing);
      }
      return { elapsedMs: 0, totalBytes: 0, updated: false };
    }

    if (!existing && !source.gpuDirty) {
      source.gpuDirty = true;
    }

    if (!source.gpuDirty) {
      existing!.syncRevision = syncRevision;
      return { elapsedMs: 0, totalBytes: 0, updated: false };
    }

    const upload = this.uploadChunkMesh(source, mesh, existing, syncRevision);
    source.gpuDirty = false;
    return {
      elapsedMs: upload.elapsedMs,
      totalBytes: upload.totalBytes,
      updated: true,
    };
  }

  private uploadChunkMesh(
    chunk: RenderMeshSource,
    mesh: ChunkMeshData,
    previous: GpuChunkResources | undefined,
    syncRevision: number,
  ): {
    elapsedMs: number;
    totalBytes: number;
  } {
    const startedAt = performance.now();
    const vertexCapacity = nextBufferCapacity(mesh.vertexData.byteLength, previous?.vertexCapacity ?? 0);
    const indexCapacity = nextBufferCapacity(mesh.indexData.byteLength, previous?.indexCapacity ?? 0);
    const waterVertexCapacity = mesh.waterVertexData.byteLength > 0
      ? nextBufferCapacity(mesh.waterVertexData.byteLength, previous?.waterVertexCapacity ?? 0)
      : 0;
    const waterIndexCapacity = mesh.waterIndexData.byteLength > 0
      ? nextBufferCapacity(mesh.waterIndexData.byteLength, previous?.waterIndexCapacity ?? 0)
      : 0;
    const vertexBuffer = !previous || previous.vertexCapacity < mesh.vertexData.byteLength
      ? this.device.createBuffer({
          size: vertexCapacity,
          usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
        })
      : previous.vertexBuffer;
    const indexBuffer = !previous || previous.indexCapacity < mesh.indexData.byteLength
      ? this.device.createBuffer({
          size: indexCapacity,
          usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST,
        })
      : previous.indexBuffer;
    const waterVertexBuffer = mesh.waterVertexData.byteLength === 0
      ? null
      : !previous || !previous.waterVertexBuffer || previous.waterVertexCapacity < mesh.waterVertexData.byteLength
      ? this.device.createBuffer({
          size: waterVertexCapacity,
          usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
        })
      : previous.waterVertexBuffer;
    const waterIndexBuffer = mesh.waterIndexData.byteLength === 0
      ? null
      : !previous || !previous.waterIndexBuffer || previous.waterIndexCapacity < mesh.waterIndexData.byteLength
      ? this.device.createBuffer({
          size: waterIndexCapacity,
          usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST,
        })
      : previous.waterIndexBuffer;

    if (vertexBuffer !== previous?.vertexBuffer) {
      previous?.vertexBuffer.destroy();
    }
    if (indexBuffer !== previous?.indexBuffer) {
      previous?.indexBuffer.destroy();
    }
    if (waterVertexBuffer !== previous?.waterVertexBuffer) {
      previous?.waterVertexBuffer?.destroy();
    }
    if (waterIndexBuffer !== previous?.waterIndexBuffer) {
      previous?.waterIndexBuffer?.destroy();
    }

    if (mesh.vertexData.byteLength > 0) {
      this.device.queue.writeBuffer(vertexBuffer, 0, mesh.vertexData);
    }
    if (mesh.indexData.byteLength > 0) {
      const indexBytes = new Uint8Array(mesh.indexData.buffer, mesh.indexData.byteOffset, mesh.indexData.byteLength);
      this.device.queue.writeBuffer(
        indexBuffer,
        0,
        indexBytes as unknown as GPUAllowSharedBufferSource,
      );
    }
    if (waterVertexBuffer && mesh.waterVertexData.byteLength > 0) {
      this.device.queue.writeBuffer(waterVertexBuffer, 0, mesh.waterVertexData);
    }
    if (waterIndexBuffer && mesh.waterIndexData.byteLength > 0) {
      const waterIndexBytes = new Uint8Array(
        mesh.waterIndexData.buffer,
        mesh.waterIndexData.byteOffset,
        mesh.waterIndexData.byteLength,
      );
      this.device.queue.writeBuffer(
        waterIndexBuffer,
        0,
        waterIndexBytes as unknown as GPUAllowSharedBufferSource,
      );
    }
    this.resources.set(chunk, {
      vertexBuffer,
      indexBuffer,
      vertexCapacity,
      indexCapacity,
      indexCount: mesh.indexCount,
      triangleCount: mesh.triangleCount - mesh.waterTriangleCount,
      waterVertexBuffer,
      waterIndexBuffer,
      waterVertexCapacity,
      waterIndexCapacity,
      waterIndexCount: mesh.waterIndexCount,
      waterTriangleCount: mesh.waterTriangleCount,
      syncRevision,
    });
    return {
      elapsedMs: performance.now() - startedAt,
      totalBytes: mesh.vertexData.byteLength
        + mesh.indexData.byteLength
        + mesh.waterVertexData.byteLength
        + mesh.waterIndexData.byteLength,
    };
  }

  private destroyResource(resourceKey: object, resource: GpuChunkResources): void {
    resource.vertexBuffer.destroy();
    resource.indexBuffer.destroy();
    resource.waterVertexBuffer?.destroy();
    resource.waterIndexBuffer?.destroy();
    this.resources.delete(resourceKey);
  }
}

function alignTo(value: number, alignment: number): number {
  return Math.ceil(value / alignment) * alignment;
}

function nextBufferCapacity(requiredBytes: number, currentCapacity: number): number {
  const minCapacity = Math.max(256, alignTo(requiredBytes, 256));
  if (currentCapacity >= minCapacity) {
    return currentCapacity;
  }
  let nextCapacity = currentCapacity > 0 ? currentCapacity : 4096;
  while (nextCapacity < minCapacity) {
    nextCapacity = nextCapacity < 65536
      ? nextCapacity * 2
      : alignTo(Math.ceil(nextCapacity * 1.5), 4096);
  }
  return nextCapacity;
}

function isCameraWithViewProjection(camera: RenderCamera): camera is { viewProjection: Float32Array } {
  return "viewProjection" in camera;
}

function resolveSkyWeatherEnvironment(renderEnvironment: RenderEnvironment): SkyWeatherEnvironment {
  const atmosphere = renderEnvironment as RenderEnvironment & Partial<SkyWeatherEnvironment>;
  const skyTopColorRgba = atmosphere.skyTopColorRgba ?? renderEnvironment.clearColorRgba;
  const skyHorizonColorRgba = atmosphere.skyHorizonColorRgba ?? renderEnvironment.clearColorRgba;
  const skyCloudColorRgba = atmosphere.skyCloudColorRgba ?? renderEnvironment.fogColorRgba;
  return {
    skyTopColorRgba,
    skyHorizonColorRgba,
    skyCloudColorRgba,
    skyCloudCoverage: clamp01(atmosphere.skyCloudCoverage ?? DEFAULT_SKY_WEATHER_ENVIRONMENT.skyCloudCoverage),
    skyCloudBand: clamp01(atmosphere.skyCloudBand ?? DEFAULT_SKY_WEATHER_ENVIRONMENT.skyCloudBand),
    ashfallIntensity: clamp01(atmosphere.ashfallIntensity ?? DEFAULT_SKY_WEATHER_ENVIRONMENT.ashfallIntensity),
    fungalGlowIntensity: clamp01(atmosphere.fungalGlowIntensity ?? DEFAULT_SKY_WEATHER_ENVIRONMENT.fungalGlowIntensity),
  };
}

function clamp01(value: number): number {
  return Math.max(0, Math.min(1, value));
}

function toGpuColor(color: readonly [number, number, number, number]): GPUColor {
  return {
    r: color[0] / 255,
    g: color[1] / 255,
    b: color[2] / 255,
    a: color[3] / 255,
  };
}
