use pbkdf2::pbkdf2_hmac;
use sha2::Sha512;

/// 使用 PBKDF2-HMAC-SHA512 从用户名与主密码派生出高强度的 Rclone 密钥与 Salt
pub fn derive_keys(username: &str, master_pass: &str) -> (String, String) {
    let norm_user = username.trim().to_lowercase();
    let norm_pass = master_pass.trim();

    // 派生主加密密码（100,000次迭代，抗彩虹表抗爆破）
    let pwd_salt = format!("{}_safevault_password_salt_v1", norm_user);
    let mut key_bytes = [0u8; 32];
    pbkdf2_hmac::<Sha512>(norm_pass.as_bytes(), pwd_salt.as_bytes(), 100_000, &mut key_bytes);
    let key = hex::encode(key_bytes);

    // 派生独立 Salt
    let salt_salt = format!("{}_safevault_salt_entropy_v1", norm_user);
    let mut salt_bytes = [0u8; 32];
    pbkdf2_hmac::<Sha512>(norm_pass.as_bytes(), salt_salt.as_bytes(), 100_000, &mut salt_bytes);
    let salt = hex::encode(salt_bytes);

    (key, salt)
}
