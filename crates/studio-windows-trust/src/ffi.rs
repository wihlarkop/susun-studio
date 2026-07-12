//! The single home for Win32 `unsafe` FFI in this crate.
//!
//! Everything here is Windows-only and returns plain safe Rust values — no
//! `windows`-crate types, handles, or pointers escape. [`verify_executable`]
//! answers two independent questions the design keeps separate: is the file's
//! signature trusted by the OS (chain trust), and who signed it (identity)?
//! Whether that signer is *allowed* is a policy decision made by the caller.
//!
//! Both Authenticode delivery mechanisms are handled: an **embedded** signature
//! in the PE (e.g. `podman.exe`) and a **catalog** signature where the file
//! carries no embedded signature and is instead vouched for by a signed system
//! catalog (e.g. `powershell.exe` in System32). Embedded is tried first; catalog
//! is the fallback.
//!
//! Unsafe is denied crate-wide; this module re-enables it narrowly. Every
//! `unsafe` block documents pointer validity, buffer sizing, handle ownership,
//! and cleanup.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Security::Cryptography::Catalog::{
    CATALOG_INFO, CryptCATAdminAcquireContext2, CryptCATAdminCalcHashFromFileHandle2,
    CryptCATAdminEnumCatalogFromHash, CryptCATAdminReleaseCatalogContext,
    CryptCATAdminReleaseContext, CryptCATCatalogInfoFromContext,
};
use windows::Win32::Security::Cryptography::{
    CERT_CONTEXT, CERT_FIND_SUBJECT_CERT, CERT_INFO, CERT_NAME_ATTR_TYPE,
    CERT_NAME_SIMPLE_DISPLAY_TYPE, CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED,
    CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED, CERT_QUERY_CONTENT_TYPE_FLAGS,
    CERT_QUERY_ENCODING_TYPE, CERT_QUERY_FORMAT_FLAG_BINARY, CERT_QUERY_OBJECT_FILE,
    CERT_SHA1_HASH_PROP_ID, CMSG_SIGNER_INFO, CMSG_SIGNER_INFO_PARAM, CertCloseStore,
    CertFindCertificateInStore, CertFreeCertificateContext, CertGetCertificateContextProperty,
    CertGetNameStringW, CryptMsgClose, CryptMsgGetParam, CryptQueryObject, HCERTSTORE,
    PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
};
use windows::Win32::Security::WinTrust::{
    WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_CATALOG_INFO, WINTRUST_DATA, WINTRUST_DATA_0,
    WINTRUST_FILE_INFO, WTD_CHOICE_CATALOG, WTD_CHOICE_FILE, WTD_REVOKE_NONE,
    WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE, WinVerifyTrust,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, OPEN_EXISTING,
};
use windows::core::{PCWSTR, w};

/// szOID_ORGANIZATION_NAME — the OID for the certificate subject organization.
const OID_ORGANIZATION_NAME: &[u8] = b"2.5.4.10\0";
/// GENERIC_READ desired-access for opening a file to hash/verify.
const GENERIC_READ: u32 = 0x8000_0000;

/// The signer identity read from a file's (or catalog's) signature. Plain owned
/// strings only — safe to hand across the crate boundary.
pub struct RawSigner {
    pub subject_common_name: String,
    pub organization: Option<String>,
    pub thumbprint_hex: String,
}

/// The result of verifying a single executable.
pub enum VerifyOutcome {
    /// Chain trust succeeded (embedded or catalog) and the signer was read.
    Trusted(RawSigner),
    /// No trusted signature (unsigned, tampered, revoked, or unverifiable).
    Untrusted,
    /// The signature is trusted but its signer certificate could not be read.
    SignerUnreadable,
}

/// Verifies a file's signature (embedded first, then catalog) and, on success,
/// reads its signer identity.
pub fn verify_executable(path: &Path) -> VerifyOutcome {
    if file_embedded_trusted(path) {
        return match extract_signer_from_file(path) {
            Some(signer) => VerifyOutcome::Trusted(signer),
            None => VerifyOutcome::SignerUnreadable,
        };
    }
    verify_via_catalog(path)
}

/// A NUL-terminated UTF-16 copy of `path`, kept alive for the duration of an FFI
/// call that borrows its pointer.
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn hex_upper(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// Whether the file carries a trusted **embedded** Authenticode signature.
/// `false` for catalog-signed files (they have no embedded signature) and for
/// unsigned/tampered/untrusted files. Always issues a matching CLOSE.
fn file_embedded_trusted(path: &Path) -> bool {
    let path_wide = wide(path);
    let mut file_info = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(path_wide.as_ptr()),
        hFile: Default::default(),
        pgKnownSubject: std::ptr::null_mut(),
    };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_FILE,
        Anonymous: WINTRUST_DATA_0 {
            pFile: &mut file_info,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    verify_and_close(&mut data)
}

/// Runs the VERIFY action then the CLOSE action on an already-populated
/// `WINTRUST_DATA`, returning whether the signature was trusted. Splitting this
/// out guarantees the state-data handle allocated by VERIFY is always freed.
fn verify_and_close(data: &mut WINTRUST_DATA) -> bool {
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: `data` (and everything it points at) outlives both calls; `action`
    // is an owned action GUID. VERIFY allocates `data.hWVTStateData`; the CLOSE
    // below frees it unconditionally.
    let status =
        unsafe { WinVerifyTrust(HWND::default(), &mut action, data as *mut _ as *mut c_void) };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: same `data`/`action`; CLOSE releases the state-data handle.
    unsafe {
        let _ = WinVerifyTrust(HWND::default(), &mut action, data as *mut _ as *mut c_void);
    }
    status == 0
}

/// Opens a file for reading (share-read), or `None` if it cannot be opened.
fn open_for_read(path: &Path) -> Option<HANDLE> {
    let path_wide = wide(path);
    // SAFETY: `path_wide` is a valid NUL-terminated wide string; all other
    // params are plain values / null. The returned handle is owned by us.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_wide.as_ptr()),
            GENERIC_READ,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };
    match handle {
        Ok(handle) if !handle.is_invalid() => Some(handle),
        _ => None,
    }
}

/// Verifies a file against the Windows security catalogs. Used for system
/// binaries whose signature is not embedded (e.g. `powershell.exe`). On success
/// the signer is read from the signed catalog file itself. Releases the file
/// handle and both catalog handles on every path.
fn verify_via_catalog(path: &Path) -> VerifyOutcome {
    let Some(file) = open_for_read(path) else {
        return VerifyOutcome::Untrusted;
    };
    let outcome = catalog_outcome(path, file);
    // SAFETY: `file` was opened by `open_for_read` and is closed exactly once.
    unsafe {
        let _ = CloseHandle(file);
    }
    outcome
}

fn catalog_outcome(path: &Path, file: HANDLE) -> VerifyOutcome {
    let mut cat_admin: isize = 0;
    // SAFETY: out-param receives an owned catalog-admin handle, released below;
    // `w!("SHA256")` is a static NUL-terminated wide string.
    let acquired =
        unsafe { CryptCATAdminAcquireContext2(&mut cat_admin, None, w!("SHA256"), None, None) };
    if acquired.is_err() || cat_admin == 0 {
        return VerifyOutcome::Untrusted;
    }
    let outcome = catalog_outcome_with_admin(path, file, cat_admin);
    // SAFETY: `cat_admin` was acquired above and is released exactly once.
    unsafe {
        let _ = CryptCATAdminReleaseContext(cat_admin, 0);
    }
    outcome
}

fn catalog_outcome_with_admin(path: &Path, file: HANDLE, cat_admin: isize) -> VerifyOutcome {
    // Compute the file's catalog hash (size, then bytes).
    let mut hash_len: u32 = 0;
    // SAFETY: `None` data pointer requests the hash length; handles are valid.
    let sized =
        unsafe { CryptCATAdminCalcHashFromFileHandle2(cat_admin, file, &mut hash_len, None, None) };
    if sized.is_err() || hash_len == 0 {
        return VerifyOutcome::Untrusted;
    }
    let mut hash = vec![0u8; hash_len as usize];
    // SAFETY: `hash` is exactly `hash_len` bytes; handles are valid.
    let hashed = unsafe {
        CryptCATAdminCalcHashFromFileHandle2(
            cat_admin,
            file,
            &mut hash_len,
            Some(hash.as_mut_ptr()),
            None,
        )
    };
    if hashed.is_err() {
        return VerifyOutcome::Untrusted;
    }

    // Find a catalog that vouches for this hash.
    // SAFETY: `hash` is a valid slice; `cat_admin` is valid. A 0 return means no
    // catalog contains the hash (the file is not catalog-signed).
    let cat_info = unsafe { CryptCATAdminEnumCatalogFromHash(cat_admin, &hash, None, None) };
    if cat_info == 0 {
        return VerifyOutcome::Untrusted;
    }
    let outcome =
        catalog_outcome_with_catalog(path, file, cat_admin, cat_info, &mut hash, hash_len);
    // SAFETY: `cat_info` was returned by the enum call and is released once.
    unsafe {
        let _ = CryptCATAdminReleaseCatalogContext(cat_admin, cat_info, 0);
    }
    outcome
}

fn catalog_outcome_with_catalog(
    path: &Path,
    file: HANDLE,
    cat_admin: isize,
    cat_info: isize,
    hash: &mut [u8],
    hash_len: u32,
) -> VerifyOutcome {
    let mut info = CATALOG_INFO {
        cbStruct: size_of::<CATALOG_INFO>() as u32,
        wszCatalogFile: [0u16; 260],
    };
    // SAFETY: `info` is a valid, sized out-param; `cat_info` is valid.
    if unsafe { CryptCATCatalogInfoFromContext(cat_info, &mut info, 0) }.is_err() {
        return VerifyOutcome::Untrusted;
    }

    // The catalog file path (NUL-terminated inside the fixed array).
    let catalog_end = info
        .wszCatalogFile
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.wszCatalogFile.len());
    let catalog_path = PathBuf::from(std::ffi::OsString::from_wide(
        &info.wszCatalogFile[..catalog_end],
    ));
    let catalog_wide = wide(&catalog_path);
    let path_wide = wide(path);
    // Member tag: the file hash as an uppercase-hex wide string.
    let mut member_tag: Vec<u16> = hex_upper(hash)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut catalog = WINTRUST_CATALOG_INFO {
        cbStruct: size_of::<WINTRUST_CATALOG_INFO>() as u32,
        dwCatalogVersion: 0,
        pcwszCatalogFilePath: PCWSTR(catalog_wide.as_ptr()),
        pcwszMemberTag: PCWSTR(member_tag.as_mut_ptr()),
        pcwszMemberFilePath: PCWSTR(path_wide.as_ptr()),
        hMemberFile: file,
        pbCalculatedFileHash: hash.as_mut_ptr(),
        cbCalculatedFileHash: hash_len,
        pcCatalogContext: std::ptr::null_mut(),
        hCatAdmin: cat_admin,
    };
    let mut data = WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwUnionChoice: WTD_CHOICE_CATALOG,
        Anonymous: WINTRUST_DATA_0 {
            pCatalog: &mut catalog,
        },
        dwStateAction: WTD_STATEACTION_VERIFY,
        ..Default::default()
    };
    if !verify_and_close(&mut data) {
        return VerifyOutcome::Untrusted;
    }

    // Trusted via catalog: the signer identity is the catalog's own signer.
    match extract_signer_from_file(&catalog_path) {
        Some(signer) => VerifyOutcome::Trusted(signer),
        None => VerifyOutcome::SignerUnreadable,
    }
}

/// Extracts the leaf signer certificate's subject CN, organization, and SHA-1
/// thumbprint from a signed file — either a PE with an embedded signature or a
/// standalone signed catalog (`.cat`). Returns `None` when no signer certificate
/// can be located. Every acquired handle is released before returning.
fn extract_signer_from_file(path: &Path) -> Option<RawSigner> {
    let path_wide = wide(path);
    let mut store = HCERTSTORE::default();
    let mut msg: *mut c_void = std::ptr::null_mut();

    // Accept both an embedded PKCS7 (PE) and a standalone PKCS7 (catalog).
    let content_flags = CERT_QUERY_CONTENT_TYPE_FLAGS(
        CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED_EMBED.0 | CERT_QUERY_CONTENT_FLAG_PKCS7_SIGNED.0,
    );

    // SAFETY: `path_wide` is a valid NUL-terminated wide string; the two out
    // params receive owned handles we free below. Unused out params are None.
    let queried = unsafe {
        CryptQueryObject(
            CERT_QUERY_OBJECT_FILE,
            path_wide.as_ptr() as *const c_void,
            content_flags,
            CERT_QUERY_FORMAT_FLAG_BINARY,
            0,
            None,
            None,
            None,
            Some(&mut store),
            Some(&mut msg),
            None,
        )
    };
    if queried.is_err() || msg.is_null() {
        return None;
    }

    let result = read_signer_from_message(store, msg);

    // SAFETY: both handles were produced by the successful CryptQueryObject
    // above and are freed exactly once here.
    unsafe {
        let _ = CryptMsgClose(Some(msg));
        let _ = CertCloseStore(Some(store), 0);
    }
    result
}

/// Reads the signer certificate out of an already-opened signed message + store.
/// Split out so the store/message cleanup in `extract_signer_from_file` runs on
/// every path.
fn read_signer_from_message(store: HCERTSTORE, msg: *mut c_void) -> Option<RawSigner> {
    // First call: ask for the signer-info blob size.
    let mut signer_info_len: u32 = 0;
    // SAFETY: `None` data pointer with a valid size out-param asks only for the
    // required byte count; `msg` is a valid message handle.
    let sized =
        unsafe { CryptMsgGetParam(msg, CMSG_SIGNER_INFO_PARAM, 0, None, &mut signer_info_len) };
    if sized.is_err() || signer_info_len == 0 {
        return None;
    }

    let mut buffer = vec![0u8; signer_info_len as usize];
    // SAFETY: `buffer` is exactly `signer_info_len` bytes, matching the size the
    // sizing call reported; `msg` is valid.
    let got = unsafe {
        CryptMsgGetParam(
            msg,
            CMSG_SIGNER_INFO_PARAM,
            0,
            Some(buffer.as_mut_ptr() as *mut c_void),
            &mut signer_info_len,
        )
    };
    if got.is_err() {
        return None;
    }

    // SAFETY: `buffer` holds a `CMSG_SIGNER_INFO` written by CryptMsgGetParam;
    // reading it back at the same layout is valid for the buffer's lifetime,
    // which outlives `cert_info` below.
    let signer_info = unsafe { &*(buffer.as_ptr() as *const CMSG_SIGNER_INFO) };

    // Find the signer's certificate in the message's store by issuer + serial.
    let cert_info = CERT_INFO {
        Issuer: signer_info.Issuer,
        SerialNumber: signer_info.SerialNumber,
        ..Default::default()
    };
    // SAFETY: `store` is a valid cert store; `cert_info` (borrowing blobs that
    // live in `buffer`) stays alive across the call. A null return means "not
    // found". The returned context is owned and freed below.
    let cert = unsafe {
        CertFindCertificateInStore(
            store,
            CERT_QUERY_ENCODING_TYPE(X509_ASN_ENCODING.0 | PKCS_7_ASN_ENCODING.0),
            0,
            CERT_FIND_SUBJECT_CERT,
            Some(&cert_info as *const _ as *const c_void),
            None,
        )
    };
    if cert.is_null() {
        return None;
    }

    let subject_common_name = cert_name_string(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, None);
    let organization = cert_attr_string(cert, OID_ORGANIZATION_NAME);
    let thumbprint_hex = cert_sha1_thumbprint(cert);

    // SAFETY: `cert` was returned owned by CertFindCertificateInStore and is
    // freed exactly once here; not used afterwards.
    unsafe {
        let _ = CertFreeCertificateContext(Some(cert as *const CERT_CONTEXT));
    }

    match (subject_common_name, thumbprint_hex) {
        (Some(cn), Some(thumb)) => Some(RawSigner {
            subject_common_name: cn,
            organization,
            thumbprint_hex: thumb,
        }),
        _ => None,
    }
}

/// Reads a certificate display/name string via `CertGetNameStringW` using the
/// two-call size-then-fill pattern.
fn cert_name_string(
    cert: *const CERT_CONTEXT,
    name_type: u32,
    type_para: Option<*const c_void>,
) -> Option<String> {
    // SAFETY: `cert` is a valid context; a `None` output buffer returns the
    // required character count (including the terminating NUL).
    let len = unsafe { CertGetNameStringW(cert, name_type, 0, type_para, None) };
    if len <= 1 {
        return None;
    }
    let mut buffer = vec![0u16; len as usize];
    // SAFETY: `buffer` is `len` wide chars, matching the reported size; `cert`
    // is valid. The call writes a NUL-terminated string into `buffer`.
    let written = unsafe { CertGetNameStringW(cert, name_type, 0, type_para, Some(&mut buffer)) };
    if written == 0 {
        return None;
    }
    // Trim the trailing NUL(s) before decoding.
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..end]))
}

/// Reads a named subject attribute (e.g. organization) by OID.
fn cert_attr_string(cert: *const CERT_CONTEXT, oid: &[u8]) -> Option<String> {
    cert_name_string(
        cert,
        CERT_NAME_ATTR_TYPE,
        Some(oid.as_ptr() as *const c_void),
    )
}

/// Reads the certificate's SHA-1 hash property and formats it as uppercase hex.
fn cert_sha1_thumbprint(cert: *const CERT_CONTEXT) -> Option<String> {
    let mut len: u32 = 0;
    // SAFETY: `None` data pointer requests the property size; `cert` is valid.
    let sized =
        unsafe { CertGetCertificateContextProperty(cert, CERT_SHA1_HASH_PROP_ID, None, &mut len) };
    if sized.is_err() || len == 0 {
        return None;
    }
    let mut buffer = vec![0u8; len as usize];
    // SAFETY: `buffer` is exactly `len` bytes; `cert` is valid.
    let got = unsafe {
        CertGetCertificateContextProperty(
            cert,
            CERT_SHA1_HASH_PROP_ID,
            Some(buffer.as_mut_ptr() as *mut c_void),
            &mut len,
        )
    };
    if got.is_err() {
        return None;
    }
    Some(hex_upper(&buffer))
}
