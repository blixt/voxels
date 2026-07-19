use serde::Serialize;
use std::error::Error;
use std::fs;
use voxels_world_service::storage_benchmark::{
    STORAGE_BENCHMARK_SCHEMA_VERSION, StorageBenchmarkRequest, run_storage_benchmark,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureResponse {
    schema_version: u32,
    error: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let request_path = arguments.next().ok_or("missing benchmark request path")?;
    let response_path = arguments.next().ok_or("missing benchmark response path")?;
    if arguments.next().is_some() {
        return Err("expected exactly a request and response path".into());
    }

    let request: StorageBenchmarkRequest = serde_json::from_slice(&fs::read(request_path)?)?;
    match run_storage_benchmark(request) {
        Ok(response) => {
            fs::write(response_path, serde_json::to_vec_pretty(&response)?)?;
            Ok(())
        }
        Err(error) => {
            fs::write(
                response_path,
                serde_json::to_vec_pretty(&FailureResponse {
                    schema_version: STORAGE_BENCHMARK_SCHEMA_VERSION,
                    error: error.to_string(),
                })?,
            )?;
            Err(error)
        }
    }
}
