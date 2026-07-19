use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use voxels_world_service::automation_fixture::{
    AUTOMATION_FIXTURE_SCHEMA_VERSION, AutomationFixtureOverlay, AutomationFixtureResolved,
    build_automation_fixture,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureRequest {
    service_source_path: PathBuf,
    client_source_path: PathBuf,
    service_output_path: PathBuf,
    client_output_path: PathBuf,
    client_output_paths: Vec<PathBuf>,
    overlay: AutomationFixtureOverlay,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureResponse {
    schema_version: u32,
    resolved: AutomationFixtureResolved,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let request_path = arguments.next().ok_or("missing fixture request path")?;
    let response_path = arguments.next().ok_or("missing fixture response path")?;
    if arguments.next().is_some() {
        return Err("expected exactly a request and response path".into());
    }

    let request: FixtureRequest = serde_json::from_slice(&fs::read(request_path)?)?;
    let fixture = build_automation_fixture(
        &fs::read_to_string(request.service_source_path)?,
        &fs::read_to_string(request.client_source_path)?,
        request.overlay,
    )?;
    fs::write(request.service_output_path, fixture.service_toml)?;
    fs::write(request.client_output_path, fixture.client_toml)?;
    if request.client_output_paths.len() != fixture.routed_client_tomls.len() {
        return Err(format!(
            "received {} routed client paths for {} configs",
            request.client_output_paths.len(),
            fixture.routed_client_tomls.len()
        )
        .into());
    }
    for (output_path, client_toml) in request
        .client_output_paths
        .into_iter()
        .zip(fixture.routed_client_tomls)
    {
        fs::write(output_path, client_toml)?;
    }
    fs::write(
        response_path,
        serde_json::to_vec_pretty(&FixtureResponse {
            schema_version: AUTOMATION_FIXTURE_SCHEMA_VERSION,
            resolved: fixture.resolved,
        })?,
    )?;
    Ok(())
}
