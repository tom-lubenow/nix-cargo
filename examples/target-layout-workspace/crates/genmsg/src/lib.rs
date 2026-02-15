include!(concat!(env!("OUT_DIR"), "/generated.rs"));

pub fn generated_message() -> &'static str {
    GENERATED_MESSAGE
}

