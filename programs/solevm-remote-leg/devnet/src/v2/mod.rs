//! The V2 instance: a fresh leg whose every strategy deployment spends a
//! P-256 risk approval, checked by the native secp256r1 program and the
//! authorization verifier. It shares no program, key, namespace or state with
//! the V1 instance, whose deployment and evidence stay as recorded.

pub(crate) mod approval;
pub(crate) mod evidence;
pub(crate) mod keys;
pub(crate) mod namespace;
pub(crate) mod report;
pub(crate) mod scenario;
pub(crate) mod settings;

use anchor_lang::prelude::Pubkey;

/// The V1 devnet test adapter, reused unchanged. V2 never deploys or
/// upgrades it; it only checks the program still runs this binary under the
/// V1 deployer.
pub(crate) const ADAPTER_SHA256: &str =
    "02a978e2e299cb089edcd8d24396e9b9579cf01d34d2b5ba58d54a3eadbd39fb";
pub(crate) const ADAPTER_ID: Pubkey =
    Pubkey::from_str_const("8bFiHDwdAurdhTv5F9JaVD4QgdJ2XxEHRK2BAyiCyjZ8");
pub(crate) const ADAPTER_AUTHORITY: Pubkey =
    Pubkey::from_str_const("HiiQJU2B2KrC5ujyrD5EJbAsA5WHyL5PwPoKmiVtKytn");

/// Programs V2 must never treat as its own leg or verifier.
pub(crate) const FOREIGN_PROGRAMS: [Pubkey; 5] = [
    // V1 devnet leg.
    Pubkey::from_str_const("27zReYgZvFTaUSWwob4qirN2Yxekcm32mnQDpMzeTyAE"),
    // V1 devnet verifier, also the verifier's default id.
    Pubkey::from_str_const("da9yz64zbrJ4fyi8VTR3xfVZu8GhfhUHkC7AZQxCGG7"),
    // Production leg and adapter.
    Pubkey::from_str_const("4sLaaRdiY74cvqVCXLjW2wPncy2ArQkTRauNTdphKByo"),
    Pubkey::from_str_const("GbFoY2KE6WAWHApg8na1Db8jtnR5cbaeCpA5TGA9UmcZ"),
    ADAPTER_ID,
];

/// Feature gates the scenario depends on, by id and name.
pub(crate) const SECP256R1_FEATURE: (&str, &str) = (
    "srremy31J5Y25FrAApwVb9kZcfXbusYMMsvTK9aWv5q",
    "SIMD-0075: enable secp256r1 precompile",
);
pub(crate) const PRECOMPILE_IN_SVM_FEATURE: (&str, &str) = (
    "9ypxGLzkMxi89eDerRKXWDXe44UY2z4hBig4mDhNq5Dp",
    "SIMD-0159: move precompile verification into SVM",
);
/// Must stay inactive: the V2 builds and the reused adapter are SBPFv0.
pub(crate) const DISABLE_SBPF_V0_EXECUTION_FEATURE: (&str, &str) = (
    "TestFeature11111111111111111111111111111111",
    "SIMD-0161: disable SBPFv0 execution",
);

/// The compiled ids must be the V2 ones, never a V1 or production id.
pub(crate) fn check_compiled_ids() -> crate::Res<()> {
    if solevm_test_strategy::ID != ADAPTER_ID {
        return Err(format!(
            "the adapter crate declares {}, not the reused devnet adapter {ADAPTER_ID}",
            solevm_test_strategy::ID
        )
        .into());
    }
    for (name, id) in [
        ("leg", solevm_remote_leg::ID),
        ("verifier", es256_authorization_verifier::ID),
    ] {
        if FOREIGN_PROGRAMS.contains(&id) {
            return Err(format!(
                "the {name} crate declares {id}, a V1 or production id; build the driver from \
                 programs/solevm-remote-leg/devnet"
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_compiles_the_v2_ids() -> crate::Res<()> {
        check_compiled_ids()?;
        assert_eq!(
            solevm_remote_leg::ID.to_string(),
            "DcbREPYFa4WtjknjmT7ENqP7RV3KAfXBLcBbGqyq2jhD"
        );
        assert_eq!(
            es256_authorization_verifier::ID.to_string(),
            "4dAHz4oRNVmQ1ru9WkUthKBKU65Bjn3jQrQ5GDfQ9CSw"
        );
        Ok(())
    }

    #[test]
    fn secp256r1_program_id_matches_the_adapter() -> crate::Res<()> {
        let instruction = es256_jws_adapter::secp256r1_instruction(&[2; 33], &[1; 64], b"m")
            .map_err(|error| format!("{error:?}"))?;
        assert_eq!(
            instruction.program_id.to_string(),
            "Secp256r1SigVerify1111111111111111111111111"
        );
        Ok(())
    }
}
