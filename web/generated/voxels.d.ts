/* tslint:disable */
/* eslint-disable */

export class EngineHandle {
    private constructor();
    free(): void;
    destroy(): void;
    feed_input(bytes: Uint8Array): boolean;
    resize(css_width: number, css_height: number, dpr: number): void;
    snapshot(): Float32Array;
}

export function create_engine(canvas: OffscreenCanvas, css_width: number, css_height: number, dpr: number, reduced_motion: boolean): Promise<EngineHandle>;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_enginehandle_free: (a: number, b: number) => void;
    readonly create_engine: (a: any, b: number, c: number, d: number, e: number) => any;
    readonly enginehandle_destroy: (a: number) => void;
    readonly enginehandle_feed_input: (a: number, b: number, c: number) => number;
    readonly enginehandle_resize: (a: number, b: number, c: number, d: number) => void;
    readonly enginehandle_snapshot: (a: number) => any;
    readonly rust_sqlite_wasm_abort: () => void;
    readonly rust_sqlite_wasm_assert_fail: (a: number, b: number, c: number, d: number) => void;
    readonly rust_sqlite_wasm_calloc: (a: number, b: number) => number;
    readonly rust_sqlite_wasm_malloc: (a: number) => number;
    readonly rust_sqlite_wasm_free: (a: number) => void;
    readonly rust_sqlite_wasm_getentropy: (a: number, b: number) => number;
    readonly rust_sqlite_wasm_localtime: (a: number) => number;
    readonly rust_sqlite_wasm_realloc: (a: number, b: number) => number;
    readonly sqlite3_os_end: () => number;
    readonly sqlite3_os_init: () => number;
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___f64______true_: (a: number, b: number, c: number) => void;
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___wasm_bindgen_7bdf24c225287f40___JsValue__core_7d5f0a2ba6a62c33___result__Result_____wasm_bindgen_7bdf24c225287f40___JsError___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___wasm_bindgen_7bdf24c225287f40___sys__JsOption_wgpu_44da0ead953be845___backend__webgpu__webgpu_sys__gen_GpuError__GpuError___core_7d5f0a2ba6a62c33___result__Result_____wasm_bindgen_7bdf24c225287f40___JsError___true_: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___wasm_bindgen_7bdf24c225287f40___sys__JsOption_wgpu_44da0ead953be845___backend__webgpu__webgpu_sys__gen_GpuError__GpuError___core_7d5f0a2ba6a62c33___result__Result_____wasm_bindgen_7bdf24c225287f40___JsError___true__7: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___wasm_bindgen_7bdf24c225287f40___sys__JsOption_wgpu_44da0ead953be845___backend__webgpu__webgpu_sys__gen_GpuError__GpuError___core_7d5f0a2ba6a62c33___result__Result_____wasm_bindgen_7bdf24c225287f40___JsError___true__8: (a: number, b: number, c: any) => [number, number];
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___js_sys_102388b01e77fd2c___Function_fn_wasm_bindgen_7bdf24c225287f40___JsValue_____wasm_bindgen_7bdf24c225287f40___sys__Undefined___js_sys_102388b01e77fd2c___Function_fn_wasm_bindgen_7bdf24c225287f40___JsValue_____wasm_bindgen_7bdf24c225287f40___sys__Undefined_______true_: (a: number, b: number, c: any, d: any) => void;
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___wasm_bindgen_7bdf24c225287f40___JsValue__wasm_bindgen_7bdf24c225287f40___JsValue__true_: (a: number, b: number, c: any) => any;
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___wasm_bindgen_7bdf24c225287f40___JsValue__wasm_bindgen_7bdf24c225287f40___JsValue__true__1: (a: number, b: number, c: any) => any;
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___wgpu_44da0ead953be845___backend__webgpu__webgpu_sys__gen_GpuDeviceLostInfo__GpuDeviceLostInfo______true_: (a: number, b: number, c: any) => void;
    readonly wasm_bindgen_7bdf24c225287f40___convert__closures_____invoke___web_sys_d83e966bf1c3c524___features__gen_MessageEvent__MessageEvent______true_: (a: number, b: number, c: any) => void;
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
