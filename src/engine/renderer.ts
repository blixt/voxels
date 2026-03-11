import { buildCameraMatrices, type CameraState } from "./camera.ts";
import type { ChunkMeshData } from "./types.ts";
import type { ResidentChunkWorld } from "./world.ts";
import { CLEAR_COLOR_RGBA, LIGHT_DIRECTION, LIGHTING_TERMS } from "./render-constants.ts";

type RenderCamera = CameraState | { viewProjection: Float32Array };

const SHADER_SOURCE = `
struct Uniforms {
  view_projection: mat4x4<f32>,
  light_direction: vec4<f32>,
  lighting_terms: vec4<f32>,
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
  @location(1) color: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
  var output: VertexOutput;
  output.clip_position = uniforms.view_projection * vec4<f32>(input.position, 1.0);
  output.normal = normalize(input.normal.xyz);
  output.color = input.color.rgb;
  return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
  let directional = max(dot(input.normal, -uniforms.light_direction.xyz), 0.0);
  let hemi = input.normal.y * 0.5 + 0.5;
  let lighting = uniforms.lighting_terms.x + uniforms.lighting_terms.y * directional + uniforms.lighting_terms.z * hemi;
  return vec4<f32>(input.color * lighting, 1.0);
}
`;

interface GpuChunkResources {
  vertexBuffer: GPUBuffer;
  indexBuffer: GPUBuffer;
  indexCount: number;
  triangleCount: number;
}

export interface RenderMeshSource {
  mesh: ChunkMeshData | null;
  gpuDirty: boolean;
}

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
  private readonly uniformBuffer: GPUBuffer;
  private readonly uniformBindGroup: GPUBindGroup;
  private readonly resources = new Map<object, GpuChunkResources>();
  private depthTexture: GPUTexture | null = null;
  private depthView: GPUTextureView | null = null;
  readonly format: GPUTextureFormat;
  readonly timestampQuerySupported: boolean;

  private constructor(canvas: HTMLCanvasElement, context: GPUCanvasContext, device: GPUDevice, format: GPUTextureFormat) {
    this.context = context;
    this.device = device;
    this.format = format;
    this.timestampQuerySupported = device.features.has("timestamp-query");
    const shaderModule = device.createShaderModule({ code: SHADER_SOURCE });
    const uniformBuffer = device.createBuffer({
      size: 96,
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
    this.pipeline = device.createRenderPipeline({
      layout: device.createPipelineLayout({ bindGroupLayouts: [bindGroupLayout] }),
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
    this.uniformBuffer = uniformBuffer;
    this.uniformBindGroup = device.createBindGroup({
      layout: bindGroupLayout,
      entries: [{ binding: 0, resource: { buffer: uniformBuffer } }],
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
    extraMeshes: readonly RenderMeshSource[] = [],
  ): RenderStats {
    this.configureCanvas(this.context.canvas as HTMLCanvasElement);
    const syncStartedAt = performance.now();
    const syncStats = this.syncResources(world, extraMeshes);
    const syncResourcesMs = performance.now() - syncStartedAt;
    const canvas = this.context.canvas as HTMLCanvasElement;
    this.writeUniforms(camera, canvas.width / canvas.height);

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
  ): Promise<ReadbackImage> {
    this.syncResources(world, extraMeshes);
    this.writeUniforms(camera, width / height);

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
  }

  private encodeRenderPass(
    world: ResidentChunkWorld,
    extraMeshes: readonly RenderMeshSource[],
    encoder: GPUCommandEncoder,
    passDescriptor: GPURenderPassDescriptor,
  ): PassStats {
    const pass = encoder.beginRenderPass(passDescriptor);
    pass.setPipeline(this.pipeline);
    pass.setBindGroup(0, this.uniformBindGroup);

    let drawCalls = 0;
    let triangles = 0;
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
    pass.end();
    return { drawCalls, triangles };
  }

  private writeUniforms(camera: RenderCamera, aspect: number): void {
    const viewProjection = "viewProjection" in camera
      ? camera.viewProjection
      : buildCameraMatrices(camera, aspect).viewProjection;
    const uniformData = new Float32Array(24);
    uniformData.set(viewProjection, 0);
    uniformData.set([...LIGHT_DIRECTION, 0], 16);
    uniformData.set([...LIGHTING_TERMS, 0], 20);
    this.device.queue.writeBuffer(this.uniformBuffer, 0, uniformData);
  }

  private syncResources(world: ResidentChunkWorld, extraMeshes: readonly RenderMeshSource[]): {
    uploadMs: number;
    uploadChunks: number;
    uploadBytes: number;
  } {
    let uploadMs = 0;
    let uploadChunks = 0;
    let uploadBytes = 0;
    const desiredResources = new Set<object>();
    for (const chunk of world.iterateResidentChunks()) {
      desiredResources.add(chunk);
    }
    for (const extraMesh of extraMeshes) {
      desiredResources.add(extraMesh);
    }

    for (const [resourceKey, resource] of this.resources.entries()) {
      if (!desiredResources.has(resourceKey)) {
        resource.vertexBuffer.destroy();
        resource.indexBuffer.destroy();
        this.resources.delete(resourceKey);
      }
    }

    for (const chunk of world.iterateResidentChunks()) {
      const mesh = chunk.mesh;
      if (!mesh || mesh.indexCount === 0) {
        const existing = this.resources.get(chunk);
        if (existing) {
          existing.vertexBuffer.destroy();
          existing.indexBuffer.destroy();
          this.resources.delete(chunk);
        }
        continue;
      }
      if (!chunk.gpuDirty) {
        continue;
      }
      const upload = this.uploadChunkMesh(chunk, mesh);
      uploadMs += upload.elapsedMs;
      uploadChunks += 1;
      uploadBytes += upload.totalBytes;
      chunk.gpuDirty = false;
    }
    for (const extraMesh of extraMeshes) {
      const mesh = extraMesh.mesh;
      if (!mesh || mesh.indexCount === 0) {
        const existing = this.resources.get(extraMesh);
        if (existing) {
          existing.vertexBuffer.destroy();
          existing.indexBuffer.destroy();
          this.resources.delete(extraMesh);
        }
        continue;
      }
      if (!extraMesh.gpuDirty) {
        continue;
      }
      const upload = this.uploadChunkMesh(extraMesh, mesh);
      uploadMs += upload.elapsedMs;
      uploadChunks += 1;
      uploadBytes += upload.totalBytes;
      extraMesh.gpuDirty = false;
    }
    return { uploadMs, uploadChunks, uploadBytes };
  }

  private uploadChunkMesh(chunk: RenderMeshSource, mesh: ChunkMeshData): {
    elapsedMs: number;
    totalBytes: number;
  } {
    const startedAt = performance.now();
    const previous = this.resources.get(chunk);
    previous?.vertexBuffer.destroy();
    previous?.indexBuffer.destroy();
    const vertexBuffer = this.device.createBuffer({
      size: mesh.vertexData.byteLength,
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
      mappedAtCreation: true,
    });
    new Uint8Array(vertexBuffer.getMappedRange()).set(new Uint8Array(mesh.vertexData));
    vertexBuffer.unmap();

    const indexBuffer = this.device.createBuffer({
      size: mesh.indexData.byteLength,
      usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST,
      mappedAtCreation: true,
    });
    new Uint8Array(indexBuffer.getMappedRange()).set(new Uint8Array(mesh.indexData.buffer));
    indexBuffer.unmap();

    this.resources.set(chunk, {
      vertexBuffer,
      indexBuffer,
      indexCount: mesh.indexCount,
      triangleCount: mesh.triangleCount,
    });
    return {
      elapsedMs: performance.now() - startedAt,
      totalBytes: mesh.vertexData.byteLength + mesh.indexData.byteLength,
    };
  }
}

function alignTo(value: number, alignment: number): number {
  return Math.ceil(value / alignment) * alignment;
}

function toGpuColor(color: readonly [number, number, number, number]): GPUColor {
  return {
    r: color[0] / 255,
    g: color[1] / 255,
    b: color[2] / 255,
    a: color[3] / 255,
  };
}
