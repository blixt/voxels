import { buildCameraMatrices, type CameraState } from "./camera.ts";
import type { ChunkMeshData } from "./types.ts";
import type { ResidentChunkWorld } from "./world.ts";
import {
  CLEAR_COLOR_RGBA,
  FOG_COLOR_RGBA,
  FOG_END_DISTANCE,
  FOG_START_DISTANCE,
  LIGHT_DIRECTION,
  LIGHTING_TERMS,
} from "./render-constants.ts";

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
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct FarFieldMask {
  meta0: vec4<i32>,
  meta1: vec4<i32>,
  words: array<u32, 32>,
}

@group(0) @binding(1) var<storage, read> far_field_mask: FarFieldMask;

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

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
  var output: VertexOutput;
  output.clip_position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
  output.normal = normalize(input.normal.xyz);
  output.color = input.color;
  output.world_position = input.position;
  return output;
}

fn shade_fragment(input: VertexOutput) -> vec4<f32> {
  let directional = max(dot(input.normal, -uniforms.light_direction.xyz), 0.0);
  let hemi = input.normal.y * 0.5 + 0.5;
  let lighting = uniforms.lighting_terms.x + uniforms.lighting_terms.y * directional + uniforms.lighting_terms.z * hemi;
  let shaded = input.color.rgb * lighting;
  let fog_distance = distance(input.world_position, uniforms.camera_position.xyz);
  let fog = smoothstep(uniforms.fog_params.x, uniforms.fog_params.y, fog_distance);
  return vec4<f32>(shaded * (1.0 - fog) + uniforms.fog_color.rgb * fog, input.color.a);
}

fn far_field_mask_contains(world_position: vec3<f32>) -> bool {
  if (far_field_mask.meta1.x == 0) {
    return false;
  }
  let span_chunks = far_field_mask.meta0.z;
  let chunk_size = f32(far_field_mask.meta0.w);
  let chunk_x = i32(floor(world_position.x / chunk_size));
  let chunk_z = i32(floor(world_position.z / chunk_size));
  let local_x = chunk_x - far_field_mask.meta0.x;
  let local_z = chunk_z - far_field_mask.meta0.y;
  if (local_x < 0 || local_z < 0 || local_x >= span_chunks || local_z >= span_chunks) {
    return false;
  }
  let bit_index = u32(local_x + local_z * span_chunks);
  let word_index = bit_index / 32u;
  let bit_offset = bit_index % 32u;
  return (far_field_mask.words[word_index] & (1u << bit_offset)) != 0u;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  return shade_fragment(input);
}

@fragment
fn fs_far(input: VertexOutput) -> @location(0) vec4<f32> {
  if (far_field_mask_contains(input.world_position)) {
    discard;
  }
  return shade_fragment(input);
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

export interface FarFieldRenderMask {
  originChunkX: number;
  originChunkZ: number;
  spanChunks: number;
  chunkSizeWorldUnits: number;
  words: Uint32Array;
}

const FAR_FIELD_MASK_WORD_COUNT = 32;
const FAR_FIELD_MASK_BUFFER_BYTES = 32 + FAR_FIELD_MASK_WORD_COUNT * 4;

interface PassStats {
  drawCalls: number;
  triangles: number;
}

export interface RenderStats {
  drawCalls: number;
  triangles: number;
  syncResourcesMs: number;
  uploadMs: number;
  uploadChunks: number;
  uploadBytes: number;
  encodeMs: number;
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

export class WebGpuVoxelRenderer {
  private readonly context: GPUCanvasContext;
  private readonly device: GPUDevice;
  private readonly pipeline: GPURenderPipeline;
  private readonly farFieldPipeline: GPURenderPipeline;
  private readonly waterPipeline: GPURenderPipeline;
  private readonly farFieldWaterPipeline: GPURenderPipeline;
  private readonly uniformBuffer: GPUBuffer;
  private readonly farFieldMaskBuffer: GPUBuffer;
  private readonly uniformBindGroup: GPUBindGroup;
  private readonly resources = new Map<object, GpuChunkResources>();
  private readonly uniformData = new Float32Array(36);
  private readonly farFieldMaskData = new ArrayBuffer(FAR_FIELD_MASK_BUFFER_BYTES);
  private readonly farFieldMaskMeta = new Int32Array(this.farFieldMaskData, 0, 8);
  private readonly farFieldMaskWords = new Uint32Array(this.farFieldMaskData, 32, FAR_FIELD_MASK_WORD_COUNT);
  private depthTexture: GPUTexture | null = null;
  private depthView: GPUTextureView | null = null;
  private resourceSyncRevision = 0;
  private lastFarFieldMask: FarFieldRenderMask | null | undefined = undefined;
  readonly format: GPUTextureFormat;
  readonly timestampQuerySupported: boolean;

  private constructor(canvas: HTMLCanvasElement, context: GPUCanvasContext, device: GPUDevice, format: GPUTextureFormat) {
    this.context = context;
    this.device = device;
    this.format = format;
    this.timestampQuerySupported = device.features.has("timestamp-query");
    const shaderModule = device.createShaderModule({ code: SHADER_SOURCE });
    const uniformBuffer = device.createBuffer({
      size: 144,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    const farFieldMaskBuffer = device.createBuffer({
      size: FAR_FIELD_MASK_BUFFER_BYTES,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST,
    });
    const bindGroupLayout = device.createBindGroupLayout({
      entries: [
        {
          binding: 0,
          visibility: GPUShaderStage.VERTEX | GPUShaderStage.FRAGMENT,
          buffer: { type: "uniform" },
        },
        {
          binding: 1,
          visibility: GPUShaderStage.FRAGMENT,
          buffer: { type: "read-only-storage" },
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
    this.farFieldPipeline = device.createRenderPipeline({
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
        entryPoint: "fs_far",
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
        depthBias: 2,
        depthBiasSlopeScale: 1,
        depthBiasClamp: 0,
      },
    });
    this.farFieldWaterPipeline = device.createRenderPipeline({
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
        entryPoint: "fs_far",
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
        depthBias: 2,
        depthBiasSlopeScale: 1,
        depthBiasClamp: 0,
      },
    });
    this.uniformBuffer = uniformBuffer;
    this.farFieldMaskBuffer = farFieldMaskBuffer;
    this.uniformBindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: uniformBuffer } },
        { binding: 1, resource: { buffer: farFieldMaskBuffer } },
      ],
    });
    this.writeFarFieldMask(null);
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
    extraMeshes: readonly RenderMeshSource[] = [],
    farFieldMask: FarFieldRenderMask | null = null,
  ): RenderStats {
    this.configureCanvas(this.context.canvas as HTMLCanvasElement);
    const syncStartedAt = performance.now();
    const syncStats = this.syncResources(world, extraMeshes);
    const syncResourcesMs = performance.now() - syncStartedAt;
    const canvas = this.context.canvas as HTMLCanvasElement;
    this.writeUniforms(camera, canvas.width / canvas.height);
    this.writeFarFieldMask(farFieldMask);

    const encoder = this.device.createCommandEncoder();
    const passDescriptor: GPURenderPassDescriptor = {
      colorAttachments: [
        {
          view: this.context.getCurrentTexture().createView(),
          loadOp: "clear",
          clearValue: toGpuColor(CLEAR_COLOR_RGBA),
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
    const stats = this.encodeRenderPass(world, extraMeshes, encoder, passDescriptor);
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
    extraMeshes: readonly RenderMeshSource[] = [],
    farFieldMask: FarFieldRenderMask | null = null,
  ): Promise<ReadbackImage> {
    this.syncResources(world, extraMeshes);
    this.writeUniforms(camera, width / height);
    this.writeFarFieldMask(farFieldMask);

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
    this.encodeRenderPass(world, extraMeshes, encoder, {
      colorAttachments: [
        {
          view: colorTexture.createView(),
          loadOp: "clear",
          clearValue: toGpuColor(CLEAR_COLOR_RGBA),
          storeOp: "store",
        },
      ],
      depthStencilAttachment: {
        view: depthTexture.createView(),
        depthLoadOp: "clear",
        depthClearValue: 1,
        depthStoreOp: "store",
      },
    });
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
    this.farFieldMaskBuffer.destroy();
  }

  private encodeRenderPass(
    world: ResidentChunkWorld,
    extraMeshes: readonly RenderMeshSource[],
    encoder: GPUCommandEncoder,
    passDescriptor: GPURenderPassDescriptor,
  ): PassStats {
    const pass = encoder.beginRenderPass(passDescriptor);
    pass.setBindGroup(0, this.uniformBindGroup);

    let drawCalls = 0;
    let triangles = 0;
    pass.setPipeline(this.farFieldPipeline);
    for (const extraMesh of extraMeshes) {
      const resource = this.resources.get(extraMesh);
      if (!resource || resource.indexCount === 0) {
        continue;
      }
      pass.setVertexBuffer(0, resource.vertexBuffer);
      pass.setIndexBuffer(resource.indexBuffer, "uint32");
      pass.drawIndexed(resource.indexCount, 1, 0, 0, 0);
      drawCalls += 1;
      triangles += resource.triangleCount;
    }
    pass.setPipeline(this.pipeline);
    for (const chunk of world.iterateResidentChunks()) {
      const resource = this.resources.get(chunk);
      if (!resource || resource.indexCount === 0) {
        continue;
      }
      pass.setVertexBuffer(0, resource.vertexBuffer);
      pass.setIndexBuffer(resource.indexBuffer, "uint32");
      pass.drawIndexed(resource.indexCount, 1, 0, 0, 0);
      drawCalls += 1;
      triangles += resource.triangleCount;
    }
    pass.setPipeline(this.farFieldWaterPipeline);
    for (const extraMesh of extraMeshes) {
      const resource = this.resources.get(extraMesh);
      if (!resource || resource.waterIndexCount === 0 || !resource.waterVertexBuffer || !resource.waterIndexBuffer) {
        continue;
      }
      pass.setVertexBuffer(0, resource.waterVertexBuffer);
      pass.setIndexBuffer(resource.waterIndexBuffer, "uint32");
      pass.drawIndexed(resource.waterIndexCount, 1, 0, 0, 0);
      drawCalls += 1;
      triangles += resource.waterTriangleCount;
    }
    pass.setPipeline(this.waterPipeline);
    for (const chunk of world.iterateResidentChunks()) {
      const resource = this.resources.get(chunk);
      if (!resource || resource.waterIndexCount === 0 || !resource.waterVertexBuffer || !resource.waterIndexBuffer) {
        continue;
      }
      pass.setVertexBuffer(0, resource.waterVertexBuffer);
      pass.setIndexBuffer(resource.waterIndexBuffer, "uint32");
      pass.drawIndexed(resource.waterIndexCount, 1, 0, 0, 0);
      drawCalls += 1;
      triangles += resource.waterTriangleCount;
    }
    pass.end();
    return { drawCalls, triangles };
  }

  private writeUniforms(camera: RenderCamera, aspect: number): void {
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
    uniformData[28] = FOG_COLOR_RGBA[0] / 255;
    uniformData[29] = FOG_COLOR_RGBA[1] / 255;
    uniformData[30] = FOG_COLOR_RGBA[2] / 255;
    uniformData[31] = FOG_COLOR_RGBA[3] / 255;
    uniformData[32] = FOG_START_DISTANCE;
    uniformData[33] = FOG_END_DISTANCE;
    uniformData[34] = 0;
    uniformData[35] = 0;
    this.device.queue.writeBuffer(this.uniformBuffer, 0, uniformData);
  }

  private writeFarFieldMask(mask: FarFieldRenderMask | null): void {
    if (mask === this.lastFarFieldMask) {
      return;
    }
    this.lastFarFieldMask = mask;
    const meta = this.farFieldMaskMeta;
    const words = this.farFieldMaskWords;
    meta.fill(0);
    words.fill(0);
    if (mask) {
      meta[0] = mask.originChunkX;
      meta[1] = mask.originChunkZ;
      meta[2] = mask.spanChunks;
      meta[3] = mask.chunkSizeWorldUnits;
      meta[4] = 1;
      words.set(mask.words.subarray(0, FAR_FIELD_MASK_WORD_COUNT));
    }
    this.device.queue.writeBuffer(this.farFieldMaskBuffer, 0, this.farFieldMaskData);
  }

  private syncResources(world: ResidentChunkWorld, extraMeshes: readonly RenderMeshSource[]): {
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
    for (const extraMesh of extraMeshes) {
      const upload = this.syncMeshSourceResource(extraMesh, extraMesh.mesh, syncRevision);
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

function toGpuColor(color: readonly [number, number, number, number]): GPUColor {
  return {
    r: color[0] / 255,
    g: color[1] / 255,
    b: color[2] / 255,
    a: color[3] / 255,
  };
}
