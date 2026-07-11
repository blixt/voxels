/* tslint:disable */
/* eslint-disable */

export class EngineHandle {
    private constructor();
    free(): void;
    destroy(): void;
    feed_input(bytes: Uint8Array): void;
    resize(css_width: number, css_height: number, dpr: number): void;
    snapshot(): Float32Array;
}

export function create_engine(canvas: OffscreenCanvas, css_width: number, css_height: number, dpr: number): Promise<EngineHandle>;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_enginehandle_free: (a: number, b: number) => void;
    readonly create_engine: (a: any, b: number, c: number, d: number) => any;
    readonly enginehandle_destroy: (a: number) => void;
    readonly enginehandle_feed_input: (a: number, b: number, c: number) => void;
    readonly enginehandle_resize: (a: number, b: number, c: number, d: number) => void;
    readonly enginehandle_snapshot: (a: number) => any;
    readonly wasm_bindgen__convert__closures_____invoke__he2c6339bddc2f20d: (a: number, b: number, c: number) => void;
    readonly wasm_bindgen__convert__closures_____invoke__h047b1d66ef70cf75: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen__convert__closures_____invoke__h01edf5ea193bf9b1: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen__convert__closures_____invoke__hd19213adbd3b2bc8: (a: number, b: number, c: any) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_destroy_closure: (a: number, b: number) => void;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
