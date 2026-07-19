/* tslint:disable */
/* eslint-disable */

export class EngineHandle {
    private constructor();
    free(): void;
    automation_contract(): string;
    destroy(): Promise<void>;
    feed_input(bytes: Uint8Array): boolean;
    /**
     * `[inventory_revision, air, grass, ..., glow_crystal]` in stable material-ID order.
     */
    inventory(): Float64Array;
    report_mission_control_copy_result(copied: boolean): void;
    resize(css_width: number, css_height: number, dpr: number): void;
    set_reduced_motion(reduced_motion: boolean): void;
    /**
     * Enters or leaves the same server-authorized spectator role exposed by World Lab.
     * Returning restores the exact local body snapshot captured on entry.
     */
    set_spectator(active: boolean): boolean;
    snapshot(): Float32Array;
    start_profile(profile_id: number): boolean;
    /**
     * `[resident, required, playable]` for the browser's canvas-only startup surface.
     */
    startup_progress(): Uint32Array;
    /**
     * Deterministic browser-harness seam for the exact gameplay dig action. The server, not
     * this API, expands the hit voxel into the fixed half-metre sphere and validates reach.
     */
    submit_dig(x: number, y: number, z: number): boolean;
    /**
     * Deterministic browser-harness seam that submits through the same server-authoritative
     * path as pointer input. It does not mutate local world state optimistically.
     */
    submit_edit(x: number, y: number, z: number, material_id: number): boolean;
    /**
     * Returns `[tile_x, tile_z, required_server_revision, accepted_server_revision,
     * resident, dirty, fingerprint_low32, fingerprint_high32, quad_count, activation_mask]`
     * for the tile containing one canonical voxel coordinate.
     */
    surface_edit_state(stride: number, x: number, z: number): Float64Array;
    take_mission_control_copy(): string | undefined;
    ui_open(): boolean;
}

export function create_engine(canvas: OffscreenCanvas, css_width: number, css_height: number, dpr: number, reduced_motion: boolean, config_toml: string, player: Array<any>): Promise<EngineHandle>;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly __externref_table_alloc: () => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbg_enginehandle_free: (a: number, b: number) => void;
    readonly __wbindgen_destroy_closure: (a: number, b: number) => void;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_start: () => void;
    readonly create_engine: (a: any, b: number, c: number, d: number, e: number, f: number, g: number, h: any) => any;
    readonly enginehandle_automation_contract: (a: number) => [number, number];
    readonly enginehandle_destroy: (a: number) => any;
    readonly enginehandle_feed_input: (a: number, b: number, c: number) => number;
    readonly enginehandle_inventory: (a: number) => [number, number];
    readonly enginehandle_report_mission_control_copy_result: (a: number, b: number) => void;
    readonly enginehandle_resize: (a: number, b: number, c: number, d: number) => void;
    readonly enginehandle_set_reduced_motion: (a: number, b: number) => void;
    readonly enginehandle_set_spectator: (a: number, b: number) => number;
    readonly enginehandle_snapshot: (a: number) => [number, number];
    readonly enginehandle_start_profile: (a: number, b: number) => number;
    readonly enginehandle_startup_progress: (a: number) => [number, number];
    readonly enginehandle_submit_dig: (a: number, b: number, c: number, d: number) => number;
    readonly enginehandle_submit_edit: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly enginehandle_surface_edit_state: (a: number, b: number, c: number, d: number) => [number, number];
    readonly enginehandle_take_mission_control_copy: (a: number) => [number, number];
    readonly enginehandle_ui_open: (a: number) => number;
    readonly memory: WebAssembly.Memory;
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
