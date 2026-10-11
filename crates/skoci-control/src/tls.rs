use chrono::{DateTime, Datelike, Duration, Utc};
use instant_acme::{
    Account, AccountCredentials, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder,
    OrderStatus, RetryPolicy,
};
use rcgen::{
    CertificateParams, DistinguishedName, DnType, Ia5String, KeyPair, SanType,
    PKCS_ECDSA_P256_SHA256,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use skoci_api::error::{SkociError, SkociResult};
use skoci_api::models::{spki_pin_from_der, CONTROL_TLS_PORT};
use skoci_api::responses::{ControlTlsInfo, ServerFingerprint};
use skoci_proxy::tls_shared::{CertSlot, HttpChallenges};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use x509_parser::parse_x509_certificate;

use crate::config::ServerConfig;
use crate::state::ControlTlsStatus;

pub fn is_global_unicast(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_loopback()
                && !v4.is_private()
                && !v4.is_link_local()
                && !v4.is_broadcast()
                && !v4.is_unspecified()
                && !v4.is_multicast()
        }
        IpAddr::V6(v6) => {
            let octets = v6.octets();
            let is_link_local = octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80;
            let is_unique_local = (octets[0] & 0xfe) == 0xfc;
            !v6.is_loopback()
                && !v6.is_unspecified()
                && !v6.is_multicast()
                && !is_link_local
                && !is_unique_local
        }
    }
}

#[derive(Clone)]
pub struct ControlTls {
    pub data_dir: PathBuf,
    pub key: Arc<KeyPair>,
    pub spki_pin: String,
    pub cert_slot: CertSlot,
    pub status: ControlTlsStatus,
    pub self_signed_cert: Arc<CertifiedKey>,
}

impl ControlTls {
    pub fn load_or_create(
        data_dir: &Path,
        public_ip: Option<IpAddr>,
        api_domain: Option<&str>,
    ) -> SkociResult<Self> {
        let tls_dir = data_dir.join("tls");
        fs::create_dir_all(&tls_dir).map_err(|e| {
            SkociError::internal(
                format!("Failed to create tls directory at {}: {e}", tls_dir.display()),
                "Check data directory write permissions",
            )
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&tls_dir, fs::Permissions::from_mode(0o750));
        }

        let key_path = tls_dir.join("control.key");
        let key = if key_path.exists() {
            let pem = fs::read_to_string(&key_path).map_err(|e| {
                SkociError::internal(
                    format!("Failed to read control.key: {e}"),
                    "Check file permissions on control.key",
                )
            })?;
            KeyPair::from_pem(&pem).map_err(|e| {
                SkociError::internal(
                    format!("Failed to parse control.key PEM: {e}"),
                    "Delete corrupted control.key to regenerate",
                )
            })?
        } else {
            let generated = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).map_err(|e| {
                SkociError::internal(
                    format!("Failed to generate ECDSA key: {e}"),
                    "Check cryptographic provider configuration",
                )
            })?;
            let pem = generated.serialize_pem();

            let mut opts = OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            opts.mode(0o600);

            let mut file = opts.open(&key_path).map_err(|e| {
                SkociError::internal(
                    format!("Failed to create control.key with mode 0600: {e}"),
                    "Check directory permissions",
                )
            })?;
            file.write_all(pem.as_bytes()).map_err(|e| {
                SkociError::internal(
                    format!("Failed to write control.key: {e}"),
                    "Check available disk space",
                )
            })?;
            generated
        };

        let key = Arc::new(key);
        let spki_pin = spki_pin_from_der(&key.public_key_der());

        let pin_path = tls_dir.join("control.pin");
        write_file_0644(&pin_path, &format!("{spki_pin}\n"))?;

        if let Some(domain) = api_domain {
            let domain_hint_path = tls_dir.join("api_domain");
            write_file_0644(&domain_hint_path, &format!("{domain}\n"))?;
        }

        if let Some(ip) = public_ip {
            write_file_0644(&tls_dir.join("public_ip"), &format!("{ip}\n"))?;
        }

        let self_signed_path = tls_dir.join("self_signed.crt");
        let (self_signed_pem, self_signed_not_after) = if self_signed_path.exists() {
            match fs::read_to_string(&self_signed_path) {
                Ok(content) => match parse_pem_certs(&content) {
                    Ok(certs) => match parse_cert_validity(certs[0].as_ref()) {
                        Ok((_, not_after)) if not_after > Utc::now() + Duration::days(30) => {
                            let has_old_default_cn = parse_x509_certificate(certs[0].as_ref())
                                .map(|(_, c)| {
                                    c.subject().iter_common_name().any(|cn| {
                                        cn.as_str().map(|s| s == "rcgen self signed cert").unwrap_or(false)
                                    })
                                })
                                .unwrap_or(false);
                            if has_old_default_cn {
                                let (pem, na) = generate_self_signed(&key, public_ip, api_domain)?;
                                write_file_0644(&self_signed_path, &pem)?;
                                (pem, na)
                            } else {
                                (content, not_after)
                            }
                        }
                        _ => {
                            let (pem, na) = generate_self_signed(&key, public_ip, api_domain)?;
                            write_file_0644(&self_signed_path, &pem)?;
                            (pem, na)
                        }
                    },
                    Err(_) => {
                        let (pem, na) = generate_self_signed(&key, public_ip, api_domain)?;
                        write_file_0644(&self_signed_path, &pem)?;
                        (pem, na)
                    }
                },
                Err(_) => {
                    let (pem, na) = generate_self_signed(&key, public_ip, api_domain)?;
                    write_file_0644(&self_signed_path, &pem)?;
                    (pem, na)
                }
            }
        } else {
            let (pem, na) = generate_self_signed(&key, public_ip, api_domain)?;
            write_file_0644(&self_signed_path, &pem)?;
            (pem, na)
        };

        let self_signed_certs = parse_pem_certs(&self_signed_pem)?;
        let self_signed_ck = Arc::new(build_certified_key(self_signed_certs, &key)?);

        let control_crt_path = tls_dir.join("control.crt");
        let (active_key, mode, not_after) = if control_crt_path.exists() {
            match fs::read_to_string(&control_crt_path) {
                Ok(content) => match parse_pem_certs(&content) {
                    Ok(certs) => match parse_cert_validity(certs[0].as_ref()) {
                        Ok((_, na)) if na > Utc::now() => match build_certified_key(certs, &key) {
                            Ok(ck) => (Arc::new(ck), "acme-ip".to_string(), Some(na)),
                            Err(_) => (
                                self_signed_ck.clone(),
                                "self-signed".to_string(),
                                Some(self_signed_not_after),
                            ),
                        },
                        _ => (
                            self_signed_ck.clone(),
                            "self-signed".to_string(),
                            Some(self_signed_not_after),
                        ),
                    },
                    Err(_) => (
                        self_signed_ck.clone(),
                        "self-signed".to_string(),
                        Some(self_signed_not_after),
                    ),
                },
                Err(_) => (
                    self_signed_ck.clone(),
                    "self-signed".to_string(),
                    Some(self_signed_not_after),
                ),
            }
        } else {
            (
                self_signed_ck.clone(),
                "self-signed".to_string(),
                Some(self_signed_not_after),
            )
        };

        let cert_slot = CertSlot::default();
        cert_slot.set(active_key);

        let status: ControlTlsStatus = Arc::new(parking_lot::RwLock::new(Some(ControlTlsInfo {
            spki_pin: spki_pin.clone(),
            mode,
            not_after,
            last_error: None,
        })));

        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            key,
            spki_pin,
            cert_slot,
            status,
            self_signed_cert: self_signed_ck,
        })
    }

    pub fn read_fingerprint(
        data_dir: &Path,
        public_ip: Option<IpAddr>,
    ) -> SkociResult<ServerFingerprint> {
        let pin_path = data_dir.join("tls").join("control.pin");
        let pin_content = match fs::read_to_string(&pin_path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(SkociError::not_found(
                    format!("Control TLS pin file not found at {}", pin_path.display()),
                    "Run 'skoci server start' first to generate the control plane TLS key and pin",
                ));
            }
            Err(e) => {
                return Err(SkociError::internal(
                    format!("Failed to read control.pin: {e}"),
                    "Check permissions on control.pin",
                ));
            }
        };

        let spki_pin = pin_content.trim().to_string();
        if spki_pin.is_empty() {
            return Err(SkociError::internal(
                "Control TLS pin file is empty",
                "Restart the Skoci server to regenerate control.pin",
            ));
        }

        let mut control_urls = Vec::new();
        if let Some(ip) = public_ip {
            let url = match ip {
                IpAddr::V4(v4) => format!("https://{v4}:{CONTROL_TLS_PORT}"),
                IpAddr::V6(v6) => format!("https://[{v6}]:{CONTROL_TLS_PORT}"),
            };
            control_urls.push(url);
        }

        let domain_hint_path = data_dir.join("tls").join("api_domain");
        if let Ok(content) = fs::read_to_string(&domain_hint_path) {
            let domain = content.trim();
            if !domain.is_empty() {
                control_urls.push(format!("https://{domain}"));
            }
        }

        Ok(ServerFingerprint {
            spki_pin,
            control_urls,
            version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }

    pub fn status(&self) -> ControlTlsStatus {
        self.status.clone()
    }
}

fn write_file_0600(path: &Path, content: &str) -> SkociResult<()> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600);
    let mut file = opts.open(path).map_err(|e| {
        SkociError::internal(
            format!("Failed to open {} with mode 0600: {e}", path.display()),
            "Check directory permissions",
        )
    })?;
    file.write_all(content.as_bytes()).map_err(|e| {
        SkociError::internal(
            format!("Failed to write to {}: {e}", path.display()),
            "Check available disk space",
        )
    })?;
    Ok(())
}

fn write_file_0644(path: &Path, content: &str) -> SkociResult<()> {
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o644);
    let mut file = opts.open(path).map_err(|e| {
        SkociError::internal(
            format!("Failed to open {} with mode 0644: {e}", path.display()),
            "Check directory permissions",
        )
    })?;
    file.write_all(content.as_bytes()).map_err(|e| {
        SkociError::internal(
            format!("Failed to write to {}: {e}", path.display()),
            "Check available disk space",
        )
    })?;
    Ok(())
}

pub fn parse_pem_certs(pem_str: &str) -> SkociResult<Vec<CertificateDer<'static>>> {
    let mut certs = Vec::new();
    for item in x509_parser::pem::Pem::iter_from_buffer(pem_str.as_bytes()) {
        let pem = item.map_err(|e| {
            SkociError::internal(
                format!("Invalid PEM data: {e}"),
                "Ensure valid PEM certificates are present",
            )
        })?;
        if pem.label == "CERTIFICATE" {
            certs.push(CertificateDer::from(pem.contents));
        }
    }
    if certs.is_empty() {
        return Err(SkociError::internal(
            "No certificates found in PEM data",
            "Verify certificate file contents",
        ));
    }
    Ok(certs)
}

pub fn parse_cert_validity(der: &[u8]) -> SkociResult<(DateTime<Utc>, DateTime<Utc>)> {
    let (_, cert) = parse_x509_certificate(der).map_err(|e| {
        SkociError::internal(
            format!("Failed to parse X.509 certificate: {e}"),
            "Ensure certificate DER is valid",
        )
    })?;
    let nb_ts = cert.validity().not_before.timestamp();
    let na_ts = cert.validity().not_after.timestamp();
    let not_before = DateTime::from_timestamp(nb_ts, 0).ok_or_else(|| {
        SkociError::internal(
            "Invalid not_before timestamp in certificate",
            "Check system clock and certificate validity",
        )
    })?;
    let not_after = DateTime::from_timestamp(na_ts, 0).ok_or_else(|| {
        SkociError::internal(
            "Invalid not_after timestamp in certificate",
            "Check system clock and certificate validity",
        )
    })?;
    Ok((not_before, not_after))
}

pub fn build_certified_key(
    certs: Vec<CertificateDer<'static>>,
    key: &KeyPair,
) -> SkociResult<CertifiedKey> {
    let key_der = key.serialize_der();
    let private_key_der = PrivateKeyDer::try_from(key_der).map_err(|e| {
        SkociError::internal(
            format!("Failed to parse private key DER: {e}"),
            "Verify private key format",
        )
    })?;
    let signing_key = rustls::crypto::ring::sign::any_supported_type(&private_key_der).map_err(|e| {
        SkociError::internal(
            format!("Unsupported signing key: {e}"),
            "Verify ring crypto provider support for ECDSA key",
        )
    })?;
    Ok(CertifiedKey::new(certs, signing_key))
}

fn generate_self_signed(
    key: &KeyPair,
    public_ip: Option<IpAddr>,
    api_domain: Option<&str>,
) -> SkociResult<(String, DateTime<Utc>)> {
    let mut sans = vec![
        SanType::IpAddress(IpAddr::from([127, 0, 0, 1])),
        SanType::DnsName(
            Ia5String::try_from("localhost")
                .map_err(|e| SkociError::internal(format!("Invalid SAN: {e}"), "Check SAN syntax"))?,
        ),
    ];
    if let Some(ip) = public_ip {
        if ip != IpAddr::from([127, 0, 0, 1]) {
            sans.push(SanType::IpAddress(ip));
        }
    }
    if let Some(domain) = api_domain {
        sans.push(SanType::DnsName(
            Ia5String::try_from(domain.to_string()).map_err(|e| {
                SkociError::internal(format!("Invalid SAN domain: {e}"), "Check domain syntax")
            })?,
        ));
    }

    let now = Utc::now();
    let next_year = now + Duration::days(365);

    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    let cn = match (public_ip, api_domain) {
        (Some(ip), _) => ip.to_string(),
        (None, Some(domain)) => domain.to_string(),
        (None, None) => "127.0.0.1".to_string(),
    };
    dn.push(DnType::CommonName, cn);
    params.distinguished_name = dn;
    params.not_before = rcgen::date_time_ymd(now.year(), now.month() as u8, now.day() as u8);
    params.not_after = rcgen::date_time_ymd(
        next_year.year(),
        next_year.month() as u8,
        next_year.day() as u8,
    );
    params.subject_alt_names = sans;

    let cert = params.self_signed(key).map_err(|e| {
        SkociError::internal(
            format!("Failed to self-sign certificate: {e}"),
            "Check key parameters",
        )
    })?;
    let pem = cert.pem();
    let (_, not_after) = parse_cert_validity(cert.der())?;
    Ok((pem, not_after))
}

async fn create_new_account(
    cfg: &ServerConfig,
    directory_url: &str,
) -> anyhow::Result<(Account, Option<AccountCredentials>)> {
    let email_vec = cfg
        .acme_email
        .as_ref()
        .map(|e| vec![format!("mailto:{e}")])
        .unwrap_or_default();
    let contact_refs: Vec<&str> = email_vec.iter().map(|s| s.as_str()).collect();
    let new_acc = NewAccount {
        contact: &contact_refs,
        terms_of_service_agreed: true,
        only_return_existing: false,
    };
    let (account, creds) = Account::builder()?
        .create(&new_acc, directory_url.to_string(), None)
        .await?;
    Ok((account, Some(creds)))
}

pub fn create_acme_ip_csr(
    public_ip: IpAddr,
    key: &KeyPair,
) -> SkociResult<rcgen::CertificateSigningRequest> {
    let mut params = CertificateParams::default();
    params.distinguished_name = DistinguishedName::new();
    params.subject_alt_names = vec![SanType::IpAddress(public_ip)];
    params.serialize_request(key).map_err(|e| {
        SkociError::internal(
            format!("Failed to serialize CSR: {e}"),
            "Check key and CSR parameters",
        )
    })
}

async fn issue_acme_ip_cert(
    cfg: &ServerConfig,
    public_ip: IpAddr,
    key: &KeyPair,
    http_challenges: &HttpChallenges,
) -> anyhow::Result<(String, DateTime<Utc>, Vec<CertificateDer<'static>>)> {
    let data_dir = &cfg.data_dir;
    let tls_dir = data_dir.join("tls");
    let acme_account_path = tls_dir.join("acme_account.json");

    let directory_url = if cfg.acme_staging {
        LetsEncrypt::Staging.url().to_string()
    } else {
        LetsEncrypt::Production.url().to_string()
    };

    let (account, creds_to_save) = if acme_account_path.exists() {
        match fs::read_to_string(&acme_account_path) {
            Ok(content) => match serde_json::from_str::<AccountCredentials>(&content) {
                Ok(creds) => match Account::builder()?.from_credentials(creds).await {
                    Ok(acc) => (acc, None),
                    Err(_) => create_new_account(cfg, &directory_url).await?,
                },
                Err(_) => create_new_account(cfg, &directory_url).await?,
            },
            Err(_) => create_new_account(cfg, &directory_url).await?,
        }
    } else {
        create_new_account(cfg, &directory_url).await?
    };

    if let Some(creds) = creds_to_save {
        let creds_json = serde_json::to_string_pretty(&creds)?;
        let _ = write_file_0600(&acme_account_path, &creds_json);
    }

    let identifiers = [Identifier::Ip(public_ip)];
    let order_req = NewOrder::new(&identifiers).profile("shortlived");
    let mut order = account.new_order(&order_req).await?;

    let mut tokens_published = Vec::new();
    let mut authz_err = None;

    {
        let mut authorizations = order.authorizations();
        while let Some(authz_res) = authorizations.next().await {
            let mut authz = match authz_res {
                Ok(a) => a,
                Err(e) => {
                    authz_err = Some(anyhow::anyhow!("Failed to retrieve authorization: {e}"));
                    break;
                }
            };
            if authz.status == instant_acme::AuthorizationStatus::Valid {
                continue;
            }
            if let Some(mut chall) = authz.challenge(ChallengeType::Http01) {
                let token = chall.token.to_string();
                let key_auth = chall.key_authorization().as_str().to_string();
                http_challenges.insert(&token, &key_auth);
                tokens_published.push(token);
                if let Err(e) = chall.set_ready().await {
                    authz_err = Some(anyhow::anyhow!("Failed to set challenge ready: {e}"));
                    break;
                }
            } else {
                authz_err = Some(anyhow::anyhow!(
                    "ACME authorization for {public_ip} has no http-01 challenge"
                ));
                break;
            }
        }
    }

    let poll_res = if authz_err.is_none() {
        order.poll_ready(&RetryPolicy::default()).await
    } else {
        Err(instant_acme::Error::Str("authorization failed"))
    };

    for token in &tokens_published {
        http_challenges.remove(token);
    }

    if let Some(err) = authz_err {
        return Err(err);
    }
    let status = poll_res?;
    if status != OrderStatus::Ready && status != OrderStatus::Valid {
        anyhow::bail!("ACME order status is not ready: {:?}", status);
    }

    let csr = create_acme_ip_csr(public_ip, key)?;
    order.finalize_csr(csr.der()).await?;

    let cert_chain_pem = order.poll_certificate(&RetryPolicy::default()).await?;
    let certs = parse_pem_certs(&cert_chain_pem)
        .map_err(|e| anyhow::anyhow!("Failed to parse issued certificate chain: {e}"))?;
    let (_, not_after) = parse_cert_validity(certs[0].as_ref())
        .map_err(|e| anyhow::anyhow!("Failed to parse certificate validity: {e}"))?;

    let crt_path = tls_dir.join("control.crt");
    let _ = write_file_0644(&crt_path, &cert_chain_pem);

    Ok((cert_chain_pem, not_after, certs))
}

pub fn start_acme_renewal(
    cfg: Arc<ServerConfig>,
    control_tls: ControlTls,
    http_challenges: HttpChallenges,
) {
    let public_ip = match cfg.public_ip {
        Some(ip) if is_global_unicast(ip) => ip,
        _ => return,
    };

    tokio::spawn(async move {
        let mut backoff = StdDuration::from_secs(300);
        let max_backoff = StdDuration::from_secs(6 * 3600);

        loop {
            let current_status = control_tls.status.read().clone();
            let should_renew = match &current_status {
                Some(info) if info.mode == "acme-ip" => match info.not_after {
                    Some(na) => {
                        let now = Utc::now();
                        let remaining = na - now;
                        remaining.num_seconds() <= 3 * 86400
                    }
                    None => true,
                },
                _ => true,
            };

            if should_renew {
                tracing::info!("Starting ACME IP certificate renewal for {public_ip}...");
                match issue_acme_ip_cert(&cfg, public_ip, &control_tls.key, &http_challenges).await
                {
                    Ok((_pem, not_after, certs)) => {
                        tracing::info!(
                            "ACME IP certificate successfully issued/renewed, valid until {not_after}"
                        );
                        if let Ok(certified_key) = build_certified_key(certs, &control_tls.key) {
                            control_tls.cert_slot.set(Arc::new(certified_key));
                        }
                        *control_tls.status.write() = Some(ControlTlsInfo {
                            spki_pin: control_tls.spki_pin.clone(),
                            mode: "acme-ip".to_string(),
                            not_after: Some(not_after),
                            last_error: None,
                        });
                        backoff = StdDuration::from_secs(300);
                        tokio::time::sleep(StdDuration::from_secs(3600)).await;
                    }
                    Err(err) => {
                        let err_msg = err.to_string();
                        tracing::warn!("ACME IP certificate renewal failed: {err_msg}");

                        let is_still_valid = {
                            let st = control_tls.status.read();
                            st.as_ref()
                                .and_then(|i| i.not_after)
                                .map(|na| na > Utc::now())
                                .unwrap_or(false)
                                && st.as_ref().map(|i| i.mode.as_str()) == Some("acme-ip")
                        };

                        if !is_still_valid {
                            control_tls.cert_slot.set(control_tls.self_signed_cert.clone());
                            let na = control_tls
                                .self_signed_cert
                                .cert
                                .first()
                                .and_then(|c| parse_cert_validity(c.as_ref()).map(|(_, na)| na).ok());
                            *control_tls.status.write() = Some(ControlTlsInfo {
                                spki_pin: control_tls.spki_pin.clone(),
                                mode: "self-signed".to_string(),
                                not_after: na,
                                last_error: Some(err_msg),
                            });
                        } else if let Some(info) = &mut *control_tls.status.write() {
                            info.last_error = Some(err_msg);
                        }

                        let sleep_dur = backoff;
                        backoff = (backoff * 2).min(max_backoff);
                        tokio::time::sleep(sleep_dur).await;
                    }
                }
            } else {
                tokio::time::sleep(StdDuration::from_secs(3600)).await;
            }
        }
    });
}

#[derive(Debug)]
struct DynamicCertResolver {
    cert_slot: CertSlot,
}

impl ResolvesServerCert for DynamicCertResolver {
    fn resolve(&self, _client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        self.cert_slot.get()
    }
}

pub async fn serve_control_tls(
    bind: SocketAddr,
    app: axum::Router,
    cert_slot: CertSlot,
) -> anyhow::Result<()> {
    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(DynamicCertResolver { cert_slot }));
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    let tls_acceptor = TlsAcceptor::from(Arc::new(server_config));
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to bind control TLS API on {bind}: {e}"))?;

    tracing::info!("Skoci pinned TLS control endpoint active at https://{bind}");

    const MAX_CONCURRENT_CONNECTIONS: usize = 256;
    const TLS_HANDSHAKE_TIMEOUT: StdDuration = StdDuration::from_secs(10);
    const HEADER_READ_TIMEOUT: StdDuration = StdDuration::from_secs(15);
    let conn_semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_CONNECTIONS));

    loop {
        let (stream, peer_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::debug!("Control TLS accept error: {e}");
                tokio::time::sleep(StdDuration::from_millis(10)).await;
                continue;
            }
        };

        let permit = match conn_semaphore.clone().try_acquire_owned() {
            Ok(p) => p,
            Err(_) => {
                tracing::warn!(
                    "Max concurrent connections ({MAX_CONCURRENT_CONNECTIONS}) reached, dropping TLS connection from {peer_addr}"
                );
                drop(stream);
                continue;
            }
        };

        let tls_acceptor = tls_acceptor.clone();
        let app = app.clone();

        tokio::spawn(async move {
            let _permit = permit;
            let tls_stream = match tokio::time::timeout(
                TLS_HANDSHAKE_TIMEOUT,
                tls_acceptor.accept(stream),
            )
            .await
            {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => {
                    tracing::debug!("Control TLS handshake failed from {peer_addr}: {e}");
                    return;
                }
                Err(_) => {
                    tracing::debug!("Control TLS handshake timed out from {peer_addr}");
                    return;
                }
            };

            let io = hyper_util::rt::TokioIo::new(tls_stream);
            let service =
                hyper::service::service_fn(move |mut req: hyper::Request<hyper::body::Incoming>| {
                    let mut app = app.clone();
                    async move {
                        req.extensions_mut()
                            .insert(axum::extract::ConnectInfo(peer_addr));
                        let req = req.map(axum::body::Body::new);
                        tower::ServiceExt::oneshot(&mut app, req).await
                    }
                });

            let mut builder = hyper_util::server::conn::auto::Builder::new(
                hyper_util::rt::TokioExecutor::new(),
            );
            builder
                .http1()
                .timer(hyper_util::rt::TokioTimer::new())
                .header_read_timeout(HEADER_READ_TIMEOUT);
            builder
                .http2()
                .timer(hyper_util::rt::TokioTimer::new())
                .keep_alive_interval(StdDuration::from_secs(30))
                .keep_alive_timeout(StdDuration::from_secs(10));

            if let Err(e) = builder.serve_connection_with_upgrades(io, service).await {
                tracing::debug!("Control TLS connection closed: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_unicast_gate() {
        assert!(!is_global_unicast("127.0.0.1".parse().unwrap()));
        assert!(!is_global_unicast("10.0.0.1".parse().unwrap()));
        assert!(!is_global_unicast("192.168.1.1".parse().unwrap()));
        assert!(!is_global_unicast("::1".parse().unwrap()));
        assert!(!is_global_unicast("fe80::1".parse().unwrap()));
        assert!(is_global_unicast("1.1.1.1".parse().unwrap()));
        assert!(is_global_unicast("203.0.113.10".parse().unwrap()));
        assert!(is_global_unicast("2606:4700:4700::1111".parse().unwrap()));
    }

    #[test]
    fn test_key_created_once_mode_and_reused() {
        let temp_dir =
            std::env::temp_dir().join(format!("skoci-tls-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let ct1 = ControlTls::load_or_create(
            &temp_dir,
            Some("1.2.3.4".parse().unwrap()),
            Some("api.example.com"),
        )
        .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(temp_dir.join("tls/control.key")).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
            let dir_meta = fs::metadata(temp_dir.join("tls")).unwrap();
            assert_eq!(dir_meta.permissions().mode() & 0o777, 0o750);
        }

        let key_pem1 = fs::read_to_string(temp_dir.join("tls/control.key")).unwrap();
        let pin1 = ct1.spki_pin.clone();

        let ct2 = ControlTls::load_or_create(
            &temp_dir,
            Some("1.2.3.4".parse().unwrap()),
            Some("api.example.com"),
        )
        .unwrap();
        let key_pem2 = fs::read_to_string(temp_dir.join("tls/control.key")).unwrap();
        let pin2 = ct2.spki_pin.clone();

        assert_eq!(key_pem1, key_pem2);
        assert_eq!(pin1, pin2);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_pin_file_matches_spki_pin_from_der() {
        let temp_dir =
            std::env::temp_dir().join(format!("skoci-tls-test-pin-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let ct = ControlTls::load_or_create(&temp_dir, None, None).unwrap();
        let pin_path = temp_dir.join("tls/control.pin");
        let pin_file_content = fs::read_to_string(&pin_path).unwrap();

        assert!(pin_file_content.ends_with('\n'));
        assert_eq!(pin_file_content.lines().count(), 1);
        assert_eq!(pin_file_content.trim(), ct.spki_pin);

        let key_pem = fs::read_to_string(temp_dir.join("tls/control.key")).unwrap();
        let key = KeyPair::from_pem(&key_pem).unwrap();
        let expected_pin = spki_pin_from_der(&key.public_key_der());
        assert_eq!(pin_file_content.trim(), expected_pin);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_self_signed_cert_carries_ip_san() {
        let temp_dir =
            std::env::temp_dir().join(format!("skoci-tls-test-san-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let public_ip: IpAddr = "203.0.113.10".parse().unwrap();
        let _ct = ControlTls::load_or_create(
            &temp_dir,
            Some(public_ip),
            Some("control.example.com"),
        )
        .unwrap();

        let cert_pem = fs::read_to_string(temp_dir.join("tls/self_signed.crt")).unwrap();
        let certs = parse_pem_certs(&cert_pem).unwrap();
        let (_, cert) = parse_x509_certificate(certs[0].as_ref()).unwrap();

        let san_ext = cert
            .subject_alternative_name()
            .unwrap()
            .expect("SAN extension present");
        let mut found_ip = false;
        let mut found_localhost = false;
        let mut found_domain = false;

        for name in &san_ext.value.general_names {
            match name {
                x509_parser::extensions::GeneralName::IPAddress(bytes) => {
                    if *bytes == [203, 0, 113, 10] {
                        found_ip = true;
                    }
                    if *bytes == [127, 0, 0, 1] {
                        found_localhost = true;
                    }
                }
                x509_parser::extensions::GeneralName::DNSName(s) => {
                    if *s == "localhost" {
                        found_localhost = true;
                    }
                    if *s == "control.example.com" {
                        found_domain = true;
                    }
                }
                _ => {}
            }
        }

        assert!(found_ip, "IP SAN for 203.0.113.10 not found");
        assert!(found_localhost, "localhost SAN not found");
        assert!(found_domain, "domain SAN for control.example.com not found");

        let cns: Vec<_> = cert
            .subject()
            .iter_common_name()
            .filter_map(|attr| attr.as_str().ok())
            .collect();
        assert_eq!(cns, vec!["203.0.113.10"]);
        for cn in &cns {
            assert_ne!(*cn, "rcgen self signed cert");
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_acme_ip_csr_has_no_common_name_and_carries_ip_san() {
        use x509_parser::prelude::*;

        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let public_ip: IpAddr = "203.0.113.10".parse().unwrap();

        let csr = create_acme_ip_csr(public_ip, &key).expect("CSR creation succeeds");

        let (rem, parsed_csr) =
            X509CertificationRequest::from_der(csr.der()).expect("CSR DER should parse");
        assert!(rem.is_empty(), "CSR DER should have no trailing bytes");

        let cri = &parsed_csr.certification_request_info;

        let cns: Vec<_> = cri
            .subject
            .iter_common_name()
            .filter_map(|attr| attr.as_str().ok())
            .collect();
        assert!(
            cns.is_empty(),
            "CSR subject must contain no common name, but found: {:?}",
            cns
        );
        assert_eq!(
            cri.subject.iter().count(),
            0,
            "CSR subject sequence must be completely empty"
        );

        let mut found_ip_san = false;
        let extensions = parsed_csr
            .requested_extensions()
            .expect("CSR must have requested extensions");
        for ext in extensions {
            if let ParsedExtension::SubjectAlternativeName(san) = ext {
                for name in &san.general_names {
                    if let GeneralName::IPAddress(bytes) = name {
                        if *bytes == [203, 0, 113, 10] {
                            found_ip_san = true;
                        }
                    }
                }
            }
        }
        assert!(found_ip_san, "Expected IP SAN 203.0.113.10 in CSR");
    }

    #[test]
    fn test_acme_ip_csr_ipv6() {
        use x509_parser::prelude::*;

        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).unwrap();
        let ipv6: IpAddr = "2001:db8::1".parse().unwrap();

        let csr = create_acme_ip_csr(ipv6, &key).expect("IPv6 CSR creation succeeds");

        let (rem, parsed_csr) =
            X509CertificationRequest::from_der(csr.der()).expect("IPv6 CSR DER should parse");
        assert!(rem.is_empty());

        let cri = &parsed_csr.certification_request_info;
        assert_eq!(cri.subject.iter().count(), 0);
        assert!(cri.subject.iter_common_name().next().is_none());

        let mut found_v6_san = false;
        let extensions = parsed_csr.requested_extensions().expect("extensions present");
        for ext in extensions {
            if let ParsedExtension::SubjectAlternativeName(san) = ext {
                for name in &san.general_names {
                    if let GeneralName::IPAddress(bytes) = name {
                        if let IpAddr::V6(v6) = ipv6 {
                            if *bytes == v6.octets() {
                                found_v6_san = true;
                            }
                        }
                    }
                }
            }
        }
        assert!(found_v6_san, "Expected IPv6 SAN in CSR");
    }

    #[test]
    fn test_read_fingerprint_roundtrip_and_missing() {
        let temp_dir =
            std::env::temp_dir().join(format!("skoci-tls-test-fp-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).unwrap();

        let err = ControlTls::read_fingerprint(&temp_dir, None).unwrap_err();
        assert_eq!(err.code, "not_found");
        assert!(!err.fix.is_empty());

        let public_ip: IpAddr = "203.0.113.10".parse().unwrap();
        let ct = ControlTls::load_or_create(
            &temp_dir,
            Some(public_ip),
            Some("api.example.com"),
        )
        .unwrap();

        let fp = ControlTls::read_fingerprint(&temp_dir, Some(public_ip)).unwrap();
        assert_eq!(fp.spki_pin, ct.spki_pin);
        assert_eq!(fp.version, env!("CARGO_PKG_VERSION"));
        assert!(fp
            .control_urls
            .contains(&format!("https://203.0.113.10:{CONTROL_TLS_PORT}")));
        assert!(fp.control_urls.contains(&"https://api.example.com".to_string()));

        let ipv6: IpAddr = "2001:db8::1".parse().unwrap();
        let fp_v6 = ControlTls::read_fingerprint(&temp_dir, Some(ipv6)).unwrap();
        assert!(fp_v6
            .control_urls
            .contains(&format!("https://[2001:db8::1]:{CONTROL_TLS_PORT}")));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
