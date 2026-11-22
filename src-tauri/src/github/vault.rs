//! Each account has one secure-store item. Replacing it rotates the pair together.
use super::Result;

pub(super) trait Vault: Send + Sync {
    fn load(&self, account: u64) -> Result<Vec<u8>>;
    fn save(&self, account: u64, data: &[u8]) -> Result<()>;
    fn remove(&self, account: u64) -> Result<()>;
}

pub(super) struct SystemVault;
pub(super) const MISSING: &str =
    "No credentials are available for this account. Reauthorize it in Accounts.";

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Output, Stdio};

    const STORE_ERROR: &str = "Linux Secret Service is unavailable or locked. Install secret-tool, unlock the login keyring, and try again.";

    fn run(action: &str, account: u64, input: Option<&[u8]>) -> Result<Output> {
        let mut command = Command::new("/usr/bin/secret-tool");
        command.arg(action);
        if action == "store" {
            command.arg("--label=Repo Uploader and Manager");
        }
        command.args([
            "application",
            "repo-uploader-and-manager",
            "github-account",
            &account.to_string(),
        ]);
        command.stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| STORE_ERROR)?;
        if let Some(bytes) = input {
            let result = child.stdin.take().ok_or(STORE_ERROR)?.write_all(bytes);
            if result.is_err() {
                let _ = child.wait();
                return Err(STORE_ERROR.into());
            }
        }
        child.wait_with_output().map_err(|_| STORE_ERROR.into())
    }

    fn missing(output: &Output) -> bool {
        output.status.code() == Some(1) && output.stderr.is_empty()
    }

    impl Vault for SystemVault {
        fn load(&self, account: u64) -> Result<Vec<u8>> {
            let output = run("lookup", account, None)?;
            if missing(&output) {
                return Err(MISSING.into());
            }
            if !output.status.success() {
                return Err(STORE_ERROR.into());
            }
            Ok(output.stdout)
        }

        fn save(&self, account: u64, data: &[u8]) -> Result<()> {
            if run("store", account, Some(data))?.status.success() {
                Ok(())
            } else {
                Err(STORE_ERROR.into())
            }
        }

        fn remove(&self, account: u64) -> Result<()> {
            let output = run("clear", account, None)?;
            if output.status.success() || missing(&output) {
                Ok(())
            } else {
                Err(STORE_ERROR.into())
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use std::{io, ptr};
    use windows_sys::Win32::{
        Foundation::ERROR_NOT_FOUND,
        Security::Credentials::{
            CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW,
            CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
        },
    };
    const STORE_ERROR: &str = "Windows Credential Manager could not access this credential. Check the current Windows login and try again.";

    fn target(account: u64) -> Vec<u16> {
        format!("Repo Uploader and Manager/GitHub/{account}")
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }

    impl Vault for SystemVault {
        fn load(&self, account: u64) -> Result<Vec<u8>> {
            let mut pointer = ptr::null_mut();
            // The name lives across the call; Windows owns the returned credential until CredFree.
            unsafe {
                if CredReadW(target(account).as_ptr(), CRED_TYPE_GENERIC, 0, &mut pointer) == 0 {
                    return Err(if io::Error::last_os_error().raw_os_error()
                        == Some(ERROR_NOT_FOUND as i32)
                    {
                        MISSING
                    } else {
                        STORE_ERROR
                    }
                    .into());
                }
                let data = if !(*pointer).CredentialBlob.is_null()
                    && (*pointer).CredentialBlobSize > 0
                    && (*pointer).CredentialBlobSize <= CRED_MAX_CREDENTIAL_BLOB_SIZE
                {
                    Ok(std::slice::from_raw_parts(
                        (*pointer).CredentialBlob,
                        (*pointer).CredentialBlobSize as usize,
                    )
                    .to_vec())
                } else {
                    Err(MISSING.into())
                };
                CredFree(pointer.cast());
                data
            }
        }
        fn save(&self, account: u64, data: &[u8]) -> Result<()> {
            if data.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
                return Err(
                    "The credential exceeds Windows Credential Manager's item size limit.".into(),
                );
            }
            let mut name = target(account);
            let credential = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: name.as_mut_ptr(),
                CredentialBlobSize: data.len() as u32,
                CredentialBlob: data.as_ptr().cast_mut(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                ..Default::default()
            };
            // CredWriteW copies the UTF-8 blob before returning.
            if unsafe { CredWriteW(&credential, 0) } == 0 {
                Err(STORE_ERROR.into())
            } else {
                Ok(())
            }
        }
        fn remove(&self, account: u64) -> Result<()> {
            if unsafe { CredDeleteW(target(account).as_ptr(), CRED_TYPE_GENERIC, 0) } == 0
                && io::Error::last_os_error().raw_os_error() != Some(ERROR_NOT_FOUND as i32)
            {
                Err(STORE_ERROR.into())
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    #[ignore = "Requires a disposable desktop keyring or isolated Wine prefix and explicit opt-in."]
    fn isolated_system_vault_round_trip() {
        assert_eq!(env::var("RUM_TEST_DISPOSABLE_VAULT").as_deref(), Ok("yes"));
        #[cfg(target_os = "linux")]
        {
            assert!(env::var("DBUS_SESSION_BUS_ADDRESS").is_ok());
            assert!(env::var("XDG_DATA_HOME").unwrap().contains("rum-vault-"));
        }
        #[cfg(target_os = "windows")]
        assert!(env::var("WINEPREFIX").unwrap().contains("rum-vault-"));

        let id = u64::MAX
            - SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros() as u64;
        let vault = SystemVault;
        assert_eq!(vault.load(id).err().as_deref(), Some(MISSING));
        let first = b"disposable example first";
        let replacement = b"disposable example second";
        vault.save(id, first).unwrap();
        assert_eq!(vault.load(id).unwrap(), first);
        vault.save(id, replacement).unwrap();
        assert_eq!(vault.load(id).unwrap(), replacement);
        vault.remove(id).unwrap();
        assert_eq!(vault.load(id).err().as_deref(), Some(MISSING));
    }
}
