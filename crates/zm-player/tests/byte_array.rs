use ruffle_core::{LoadBehavior, PlayerBuilder, backend::log::LogBackend, tag_utils::SwfMovie};
use std::sync::{Arc, Mutex};

struct TestLog(Arc<Mutex<Vec<String>>>);
impl LogBackend for TestLog {
    fn avm_warning(&self, message: &str) {
        panic!("AVM warning: {message}");
    }
    fn avm_trace(&self, message: &str) {
        self.0.lock().unwrap().push(message.to_owned());
    }
}

#[test]
fn byte_array_compression_preserves_data_settings_positions_and_errors() {
    for behavior in [LoadBehavior::Blocking, zm_player::game_load_behavior()] {
        let log = Arc::new(Mutex::new(Vec::new()));
        let movie = SwfMovie::from_data(
            include_bytes!("fixtures/ByteArrayRoundTrip.swf"),
            "file:///ByteArrayRoundTrip.swf".into(),
            None,
            None,
        )
        .unwrap();
        let player = PlayerBuilder::new()
            .with_movie(movie)
            .with_log(TestLog(log.clone()))
            .with_autoplay(true)
            .with_load_behavior(behavior)
            .build();
        for _ in 0..3 {
            player.lock().unwrap().run_frame();
        }
        assert_eq!(*log.lock().unwrap(), ["BYTEARRAY_ROUNDTRIP_OK"]);
    }
}
