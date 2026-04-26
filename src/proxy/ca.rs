use anyhow::{Context, Result};
use hudsucker::{
    certificate_authority::RcgenAuthority,
    rcgen::{
        BasicConstraints, CertificateParams, DistinguishedName, DnType, IsCa, Issuer, KeyPair,
        KeyUsagePurpose,
    },
    rustls,
};
use std::path::Path;
use tracing::info;

const CACHE_SIZE: u64 = 1_000;
const BASENAME: &str = "akagi-ca";
const CERT_PEM_EXTS: &[&str] = &["cer", "crt", "pem"];
const CERT_DER_EXT: &str = "der";
const KEY_PEM_EXT: &str = "key";
const KEY_DER_EXT: &str = "key.der";

pub fn load_or_generate(dir: &Path) -> Result<RcgenAuthority> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create CA dir {}", dir.display()))?;

    let cert_pem_path = dir.join(format!("{BASENAME}.cer"));
    let key_pem_path = dir.join(format!("{BASENAME}.{KEY_PEM_EXT}"));

    let (cert_pem, key_pem) = if cert_pem_path.exists() && key_pem_path.exists() {
        info!("Loading CA from {}", dir.display());
        let cert_pem =
            std::fs::read_to_string(&cert_pem_path).context("Failed to read CA cert")?;
        let key_pem = std::fs::read_to_string(&key_pem_path).context("Failed to read CA key")?;
        let key_pair_for_der =
            KeyPair::from_pem(&key_pem).context("Failed to parse CA key")?;
        write_extra_pem_formats(dir, &cert_pem)?;
        write_extra_key_der(dir, &key_pair_for_der.serialize_der())?;
        (cert_pem, key_pem)
    } else {
        info!("Generating new CA at {}", dir.display());
        let (cert_pem, key_pem, cert_der, key_der) = generate_ca()?;
        std::fs::write(&cert_pem_path, &cert_pem).context("Failed to write CA cert")?;
        std::fs::write(&key_pem_path, &key_pem).context("Failed to write CA key")?;
        write_extra_pem_formats(dir, &cert_pem)?;
        write_cert_der(dir, &cert_der)?;
        write_extra_key_der(dir, &key_der)?;
        (cert_pem, key_pem)
    };

    let key_pair = KeyPair::from_pem(&key_pem).context("Failed to parse CA key")?;
    let issuer = Issuer::from_ca_cert_pem(&cert_pem, key_pair)
        .context("Failed to parse CA certificate")?;

    Ok(RcgenAuthority::new(
        issuer,
        CACHE_SIZE,
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
}

fn write_extra_pem_formats(dir: &Path, cert_pem: &str) -> Result<()> {
    for ext in CERT_PEM_EXTS {
        let p = dir.join(format!("{BASENAME}.{ext}"));
        if !p.exists() {
            std::fs::write(&p, cert_pem)
                .with_context(|| format!("Failed to write CA cert {}", p.display()))?;
        }
    }
    Ok(())
}

fn write_cert_der(dir: &Path, cert_der: &[u8]) -> Result<()> {
    let p = dir.join(format!("{BASENAME}.{CERT_DER_EXT}"));
    if !p.exists() {
        std::fs::write(&p, cert_der)
            .with_context(|| format!("Failed to write CA cert {}", p.display()))?;
    }
    Ok(())
}

fn write_extra_key_der(dir: &Path, key_der: &[u8]) -> Result<()> {
    let p = dir.join(format!("{BASENAME}.{KEY_DER_EXT}"));
    if !p.exists() {
        std::fs::write(&p, key_der)
            .with_context(|| format!("Failed to write CA key {}", p.display()))?;
    }
    Ok(())
}

fn generate_ca() -> Result<(String, String, Vec<u8>, Vec<u8>)> {
    let key_pair = KeyPair::generate().context("Failed to generate CA key pair")?;

    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Akagi Proxy CA");
    dn.push(DnType::OrganizationName, "Akagi");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];

    let cert = params
        .self_signed(&key_pair)
        .context("Failed to self-sign CA certificate")?;

    let cert_pem = cert.pem();
    let cert_der = cert.der().to_vec();
    let key_pem = key_pair.serialize_pem();
    let key_der = key_pair.serialize_der();
    Ok((cert_pem, key_pem, cert_der, key_der))
}
