use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use lumbridge_voice::adapter::{MockTranscriber, Transcriber};
use lumbridge_voice::config::Config;
use lumbridge_voice::server::{router, App};
use lumbridge_voice::speak::HttpSpeak;
use lumbridge_voice::vllm::Vllm;

const USAGE: &str = "\
lumbridge-voice: the speech-to-text HTTP front

Usage: lumbridge-voice [--config FILE] [--print-config]

Settings come from built-in defaults, then FILE (TOML; also $LUMBRIDGE_VOICE_CONFIG),
then LUMBRIDGE_VOICE_* in the environment. See lumbridge-voice.example.toml.";

fn main() -> ExitCode {
    let mut file = std::env::var("LUMBRIDGE_VOICE_CONFIG")
        .ok()
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let mut print = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => match args.next() {
                Some(path) => file = Some(PathBuf::from(path)),
                None => return fail("--config needs a file"),
            },
            "--print-config" => print = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other => return fail(&format!("unknown argument {other}\n\n{USAGE}")),
        }
    }
    let config = match Config::load(file.as_deref(), |name| std::env::var(name).ok()) {
        Ok(c) => c,
        Err(e) => return fail(&e),
    };
    if print {
        println!("{config:#?}");
        return ExitCode::SUCCESS;
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let served = match config.adapter.as_str() {
        "mock" => runtime.block_on(serve(MockTranscriber::default(), &config)),
        _ => runtime.block_on(serve(
            Vllm::new(&config.transcribe.url, &config.transcribe.model),
            &config,
        )),
    };
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

async fn serve<T: Transcriber>(transcriber: T, config: &Config) -> Result<(), String> {
    let app = Arc::new(App {
        transcriber,
        speak: HttpSpeak::new(&config.speak.url),
        ffmpeg: config.ffmpeg.clone(),
        max_upload_bytes: config.max_upload_mb * 1024 * 1024,
    });
    let listener = tokio::net::TcpListener::bind(&config.listen)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", config.listen))?;
    eprintln!(
        "lumbridge-voice listening on {} (adapter {}, model {})",
        config.listen,
        config.adapter,
        app.transcriber.model()
    );
    axum::serve(listener, router(app))
        .await
        .map_err(|e| e.to_string())
}

fn fail(message: &str) -> ExitCode {
    eprintln!("lumbridge-voice: {message}");
    ExitCode::FAILURE
}
