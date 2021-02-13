use paperclip::{
    api_v2_schema,
    v2::{
        self,
        codegen::{DefaultEmitter, Emitter, EmitterState},
        models::ResolvableApi,
    },
};
use serde::Deserialize;
use std::{
    env,
    fs::{create_dir_all, File},
    io::Write,
    path::PathBuf,
};

#[api_v2_schema]
#[derive(Debug, Deserialize)]
struct SchemaWithExamples {}

#[allow(clippy::field_reassign_with_default)]
fn main() {
    // taken from https://github.com/launchdarkly/ld-openapi/blob/master/spec/definitions.yaml
    let fd = File::open("launchdarkly-defs.yaml").expect("missing schema");
    println!("cargo:rerun-if-changed=launchdarkly-defs.yaml");
    let raw: ResolvableApi<SchemaWithExamples> = v2::from_reader(fd).expect("deserializing spec");
    let schema = raw.resolve().expect("resolve schema");

    let cargo_out_dir = env::var("OUT_DIR").unwrap();
    let out_dir = PathBuf::from(cargo_out_dir).join("models");
    create_dir_all(&out_dir).expect("create out dir");

    let mut state = EmitterState::default();
    state.mod_prefix = "crate::models::";
    state.working_dir = out_dir.clone();

    let emitter = DefaultEmitter::from(state);
    emitter.generate(&schema).expect("models");

    // WARNING: heavy hackery
    // overwrite generated module
    //
    // paperclip includes a client implementation that we don't need
    // the only way to get rid of it is by making a new module that
    // only includes the definition structs
    let mut modf = File::create(out_dir.join("mod.rs")).expect("create file");
    for listing in out_dir.read_dir().expect("failed reading out dir") {
        let path = listing.expect("failed reading dir").path();
        let name = path.file_stem().unwrap().to_str().unwrap();
        if name == "mod" {
            continue;
        }
        // Load the serde derive macros for all modules except `util` & `generics`
        let use_insert = Some(name)
            .filter(|n| *n != "util" && *n != "generics")
            .map(|_| "    use serde::{Deserialize, Serialize};\n")
            .unwrap_or_default();
        write!(
            modf,
            r#"
pub mod {0} {{
    {1}include!("./{0}.rs");
}}
"#,
            name, use_insert
        )
        .unwrap();
    }
}
