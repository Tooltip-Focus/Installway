// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Gaëtan Dezeiraud, Louis Pinaud

use crate::args::KeygenArgs;
use anyhow::{Context, Result};
use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use std::fs;

pub fn run(args: &KeygenArgs) -> Result<()> {
    fs::create_dir_all(&args.out).context("create out dir")?;

    let mut csprng = OsRng;
    let signing = SigningKey::generate(&mut csprng);
    let verifying = signing.verifying_key();

    let priv_path = args.out.join("priv.key");
    let pub_path = args.out.join("pub.key");

    fs::write(&priv_path, hex::encode(signing.to_bytes()))
        .with_context(|| format!("write {}", priv_path.display()))?;
    fs::write(&pub_path, hex::encode(verifying.to_bytes()))
        .with_context(|| format!("write {}", pub_path.display()))?;

    println!("Wrote {}", priv_path.display());
    println!("Wrote {}", pub_path.display());
    println!();
    println!("KEEP priv.key SECRET. Lose it and every installer signed with it must be re-issued.");
    Ok(())
}
