/* @ts-self-types="./micropuffer.d.ts" */
import * as wasm from "./micropuffer_bg.wasm";
import { __wbg_set_wasm } from "./micropuffer_bg.js";

__wbg_set_wasm(wasm);
wasm.__wbindgen_start();
export {
    micropuffer_delete_namespace, micropuffer_explain_query, micropuffer_export_namespace, micropuffer_list_namespaces, micropuffer_metadata, micropuffer_patch_metadata, micropuffer_query, micropuffer_recall, micropuffer_schema, micropuffer_update_schema, micropuffer_warm_cache, micropuffer_write
} from "./micropuffer_bg.js";
