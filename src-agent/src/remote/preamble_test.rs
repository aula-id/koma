use tokio::io::AsyncReadExt;

use super::*;

#[test]
fn strip_drops_shell_noise_before_the_marker() {
    let noisy = "Welcome to Ubuntu\nnvm: using node 24\n\nKOMA-STREAM-BEGIN\n{\"sessions\":[]}";
    assert_eq!(strip(noisy), "{\"sessions\":[]}");
    assert_eq!(strip("koma 0.4.15"), "koma 0.4.15");
}

#[test]
fn sync_skip_stops_right_after_the_marker() {
    let mut input: &[u8] = b"banner\nno trailing newline\nKOMA-STREAM-BEGIN\n{\"k\":1}\n";
    skip_sync(&mut input).unwrap();
    assert_eq!(input, b"{\"k\":1}\n");
    let mut closed: &[u8] = b"banner only\n";
    assert!(skip_sync(&mut closed).is_err());
}

#[tokio::test]
async fn async_skip_handles_split_marker_and_binary_frames() {
    let (mut writer, reader) = tokio::io::duplex(64);
    let mut skip = PreambleSkip::new(reader);
    let reading = tokio::spawn(async move {
        let mut out = Vec::new();
        skip.read_to_end(&mut out).await.unwrap();
        out
    });
    use tokio::io::AsyncWriteExt;
    // A partial marker ("KOMA-") inside the noise must not confuse the match.
    for part in [
        &b"KOMA-motd KKOMA-STREAM-BE"[..],
        b"GIN\n",
        &[0, 0, 0, 2],
        b"{}",
    ] {
        writer.write_all(part).await.unwrap();
    }
    drop(writer);
    assert_eq!(reading.await.unwrap(), [0, 0, 0, 2, b'{', b'}']);
}
