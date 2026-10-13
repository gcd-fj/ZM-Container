use ruffle_core::{LoadBehavior, PlayerBuilder, backend::log::LogBackend, tag_utils::SwfMovie};
use std::io::{self, Write};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct AvmLog(Arc<Mutex<Vec<String>>>);
impl LogBackend for AvmLog {
    fn avm_warning(&self, message: &str) {
        panic!("Unexpected AVM warning: {message}");
    }
    fn avm_trace(&self, message: &str) {
        self.0.lock().unwrap().push(message.to_owned());
    }
}

#[test]
fn slow_callbacks_capture_code_stacks_without_values_or_semantic_changes() {
    for enabled in [false, true] {
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(if enabled { "off,zm_perf=info" } else { "off" })
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        let _subscriber = tracing::subscriber::set_default(subscriber);
        let avm_log = Arc::new(Mutex::new(Vec::new()));
        let movie = SwfMovie::from_data(
            include_bytes!("fixtures/SlowCallbackTrace.swf"),
            "file:///DO_NOT_LOG_MOVIE_URL_37494.swf".into(),
            None,
            None,
        )
        .unwrap();
        let player = PlayerBuilder::new()
            .with_movie(movie)
            .with_log(AvmLog(avm_log.clone()))
            .with_autoplay(true)
            .with_load_behavior(LoadBehavior::Blocking)
            .build();
        for _ in 0..3 {
            player.lock().unwrap().run_frame();
        }
        assert_eq!(*avm_log.lock().unwrap(), ["SLOW_CALLBACK_TRACE_OK"]);
        let output = String::from_utf8(capture.0.lock().unwrap().clone()).unwrap();
        assert!(!output.contains("DO_NOT_LOG"));
        if enabled {
            assert!(output.contains("AVM2 callback stack sample"), "{output}");
            assert!(output.contains("SlowCallbackTrace/busyWork"), "{output}");
            assert!(output.contains("Slow AVM2 native call"), "{output}");
            assert!(output.contains("SlowCallbackTrace/onFrame"), "{output}");
        } else {
            assert!(output.is_empty(), "{output}");
        }
    }
}
