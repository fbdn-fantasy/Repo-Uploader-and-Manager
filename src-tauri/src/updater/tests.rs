use super::*;
use serde_json::json;
use std::net::TcpListener;

fn release(tag: &str) -> Release {
    Release {
        tag_name: tag.into(),
        draft: false,
        prerelease: false,
        assets: Vec::new(),
    }
}

fn asset(name: &str, content: &[u8]) -> Asset {
    Asset {
        id: 123,
        name: name.into(),
        size: content.len() as u64,
        state: "uploaded".into(),
        digest: Some(format!("sha256:{}", hex::encode(Sha256::digest(content)))),
    }
}

#[test]
fn selects_newest_published_semver_from_release_api() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = server.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let (mut client, _) = server.accept().unwrap();
        let mut buffer = [0; 1024];
        let len = client.read(&mut buffer).unwrap();
        assert!(String::from_utf8_lossy(&buffer[..len])
            .starts_with("GET /releases?per_page=100&page=1 HTTP/1.1"));
        let entries = [
            ("V3", false, false),
            ("v1.0.9", false, false),
            ("V2.0.0", true, false),
            ("v3.0.0", false, true),
            ("1.9.0-beta.1", false, false),
            ("V1.0.10", false, false),
        ]
        .map(
            |(tag, draft, pre)| json!({"tag_name":tag,"draft":draft,"prerelease":pre,"assets":[]}),
        );
        let body = serde_json::to_vec(&entries).unwrap();
        write!(client, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        client.write_all(&body).unwrap();
    });
    let (found, _) = newest(
        &ureq::Agent::new_with_defaults(),
        &format!("http://{address}/releases"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(found, Version::new(1, 0, 10));
    thread.join().unwrap();
}

#[test]
fn exact_platform_assets_and_github_digest_required() {
    let mut release = release("V1.0.1");
    let version = release.stable_version().unwrap();
    let formats = [Format::Nsis, Format::Deb, Format::Rpm, Format::Tar];
    let names = [
        "Repo-Uploader-and-Manager-V1.0.1-Setup.exe",
        "Repo-Uploader-and-Manager-V1.0.1.deb",
        "repo-uploader-and-manager-1.0.1-1.x86_64.rpm",
        "Repo-Uploader-and-Manager-V1.0.1.tar.gz",
    ];
    release.assets.push(asset(
        "Repo-Uploader-and-Manager-V1.0.1-Windows-Source.zip",
        b"archive",
    ));
    assert!(release.asset(&version, Format::Nsis).is_err());
    for (format, name) in formats.into_iter().zip(names) {
        release.assets.push(asset(name, b"installer"));
        assert_eq!(release.asset(&version, format).unwrap().name, name);
    }
    release.assets.last_mut().unwrap().digest = None;
    assert!(release.asset(&version, Format::Tar).is_err());
    release.assets.push(asset(names[0], b"duplicate"));
    assert!(release.asset(&version, Format::Nsis).is_err());
}

#[test]
fn downloaded_bytes_must_match_published_size_and_hash() {
    let input = b"test installer";
    let asset = asset("setup.exe", input);
    let mut output = Vec::new();
    verified_copy(input.as_slice(), &mut output, &asset).unwrap();
    assert_eq!(input.as_slice(), output);
    for corrupt in [
        b"test Installer".as_slice(),
        b"test",
        b"test installer with extras",
    ] {
        assert!(verified_copy(corrupt, std::io::sink(), &asset).is_err());
    }
}
