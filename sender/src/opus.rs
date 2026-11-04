use opus_rs::{Application, OpusDecoder, OpusEncoder};

pub fn do_opus() {
    let mut encoder = OpusEncoder::new(16000, 1, Application::Voip).unwrap();
    encoder.bitrate_bps = 16000;
    encoder.use_cbr = true;

    let input = [0.0f32; 320]; // 20ms frame at 16kHz
    let mut output = [0u8; 256];
    let bytes = encoder.encode(&input, 320, &mut output).unwrap();

    // Encode PCM16 directly (issue #28): skip the f32 round-trip in the
    // SILK/VoIP path. Output is byte-for-byte identical to `encode()` called
    // with the same audio converted via `sample as f32 / 32768.0`.
    // let input_i16 = [0i16; 320]; // 20ms frame at 16kHz
    // let bytes = encoder.encode_i16(&input_i16, 320, &mut output).unwrap();

    // Decode
    let mut decoder = OpusDecoder::new(16000, 1).unwrap();
    let mut pcm = [0.0f32; 320];
    let samples = decoder.decode(&output[..bytes], 320, &mut pcm).unwrap();
    println!("Hello, world!, {:?}", bytes);
}
