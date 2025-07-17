use bitcoin::secp256k1::rand;
use frost_secp256k1_tr::rand_core::RngCore;
use k256::{
    AffinePoint, ProjectivePoint, PublicKey, Scalar,
    elliptic_curve::{PrimeField, generic_array::GenericArray},
};

use crate::signer::{SecretShare, SignerError, VerifiableSecretShare};

/// Represents a polynomial used for Shamir's Secret Sharing.
/// The coefficients are ordered from lowest to highest degree:
/// coefficients[0] is the constant term (the secret),
/// coefficients[1] is the coefficient of x, and so on.
///
/// Each coefficient has a corresponding proof (public key) that can be
/// used to verify shares without revealing the polynomial.
#[derive(Clone)]
struct Polynomial {
    /// Coefficients of the polynomial, starting with the constant term
    coefficients: Vec<Scalar>,

    /// Proofs for each coefficient (public keys derived from the coefficients)
    pub proofs: Vec<PublicKey>,
}

impl Polynomial {
    /// Evaluates the polynomial at a given point using Horner's method
    ///
    /// This implementation uses Horner's method which is more efficient than
    /// direct evaluation, requiring fewer multiplications.
    ///
    /// # Arguments
    /// * `x` - The point at which to evaluate the polynomial
    ///
    /// # Returns
    /// The value of the polynomial at point x
    pub fn evaluate(&self, x: &Scalar) -> Scalar {
        // Using Horner's method: a_n * x^n + ... + a_1 * x + a_0
        // = a_0 + x * (a_1 + x * (a_2 + ... + x * a_n))
        let mut result = Scalar::ZERO;

        // Iterate in reverse to apply Horner's method
        for coeff in self.coefficients.iter().rev() {
            result = result * *x + *coeff;
        }

        result
    }
}

/// Converts a byte array into a k256 Scalar value
///
/// # Arguments
/// * `bytes` - The 32-byte array to convert
///
/// # Returns
/// * `Ok(Scalar)` - The scalar value if conversion succeeds
/// * `Err(SignerError)` - If input is not 32 bytes or conversion fails
pub fn from_bytes_to_scalar(bytes: &[u8]) -> Result<Scalar, SignerError> {
    if bytes.len() != 32 {
        return Err(SignerError::SecretSharingError(format!(
            "Invalid byte length for scalar. Expected 32, got {}",
            bytes.len()
        )));
    }

    let arr = GenericArray::clone_from_slice(bytes);

    Scalar::from_repr_vartime(arr).ok_or_else(|| {
        SignerError::SecretSharingError("Failed to convert bytes to scalar".to_string())
    })
}

/// Splits a secret into multiple verifiable shares using Shamir's Secret Sharing
///
/// This implementation creates shares with verifiable proofs that allow recipients
/// to validate shares without revealing the original secret.
///
/// # Arguments
/// * `secret_scalar` - The secret to be shared
/// * `threshold` - The minimum number of shares needed to reconstruct the secret (t)
/// * `number_of_shares` - The total number of shares to generate (n)
///
/// # Returns
/// * `Ok(Vec<VerifiableSecretShare>)` - The generated verifiable shares
/// * `Err(SignerError)` - If parameters are invalid or share generation fails
pub fn split_secret_with_proofs(
    secret_scalar: &Scalar,
    threshold: usize,
    number_of_shares: usize,
) -> Result<Vec<VerifiableSecretShare>, SignerError> {
    // Validate inputs
    if threshold == 0 || threshold > number_of_shares {
        return Err(SignerError::SecretSharingError(format!(
            "Threshold ({threshold}) must be greater than 0 and less than or equal to the number of shares ({number_of_shares})"
        )));
    }

    // Generate the polynomial using the secret as the constant term
    let polynomial = generate_polynomial_for_secret_sharing(secret_scalar, threshold)?;

    // Generate the shares by evaluating the polynomial at distinct non-zero points
    let mut shares = Vec::with_capacity(number_of_shares);
    for i in 1..=number_of_shares {
        let index = Scalar::from(i as u64);
        let share = polynomial.evaluate(&index);
        shares.push(VerifiableSecretShare {
            secret_share: SecretShare {
                threshold,
                index,
                share,
            },
            proofs: polynomial.proofs.clone(),
        });
    }

    Ok(shares)
}

/// Creates a random polynomial with the secret as the constant term
///
/// # Arguments
/// * `secret` - The secret to be shared (becomes the constant term of the polynomial)
/// * `threshold` - The minimum number of shares needed to reconstruct the secret (t)
///
/// # Returns
/// * `Ok(Polynomial)` - The generated polynomial with corresponding proofs
/// * `Err(SignerError)` - If polynomial generation fails
fn generate_polynomial_for_secret_sharing(
    secret: &Scalar,
    threshold: usize,
) -> Result<Polynomial, SignerError> {
    let mut rng = rand::thread_rng();
    let mut coefficients = Vec::with_capacity(threshold);
    let mut proofs = Vec::with_capacity(threshold);
    // The degree of the polynomial (t-1)
    let degree = threshold - 1;

    // Set the constant term (secret)
    coefficients.push(*secret);

    // Generate proof for secret
    proofs.push(scalar_to_pubkey(secret)?);

    // Generate random coefficients for higher terms
    for _ in 1..=degree {
        let mut random_bytes = [0u8; 32];
        rng.fill_bytes(&mut random_bytes);

        // Convert to scalar and ensure it's within field modulus
        let random_scalar = from_bytes_to_scalar(&random_bytes)?;
        coefficients.push(random_scalar);
        proofs.push(scalar_to_pubkey(&random_scalar)?);
    }

    Ok(Polynomial {
        coefficients,
        proofs,
    })
}

/// Converts a scalar (private key) to its corresponding public key
///
/// This function computes the public key for a given scalar by
/// multiplying the scalar with the generator point on the elliptic curve.
///
/// # Arguments
/// * `secret` - The scalar (private key) to convert
///
/// # Returns
/// * `Ok(PublicKey)` - The corresponding public key
/// * `Err(SignerError)` - If conversion fails
fn scalar_to_pubkey(secret: &k256::Scalar) -> Result<PublicKey, SignerError> {
    let point = ProjectivePoint::GENERATOR * *secret;
    PublicKey::from_affine(AffinePoint::from(point)).map_err(|_| {
        SignerError::SecretSharingError("Failed to convert scalar to public key".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_bytes_to_scalar() {
        // Test valid conversion
        let bytes = [42u8; 32];
        let result = from_bytes_to_scalar(&bytes);
        assert!(result.is_ok());

        // Test invalid length
        let invalid_bytes = [42u8; 16]; // Only 16 bytes instead of 32
        let result = from_bytes_to_scalar(&invalid_bytes);
        assert!(result.is_err());
        match result {
            Err(SignerError::SecretSharingError(msg)) => {
                assert!(msg.contains("Invalid byte length for scalar"));
            }
            _ => panic!("Expected SecretSharingError"),
        }
    }

    #[test]
    fn test_scalar_to_pubkey() {
        // Create a known scalar
        let scalar_bytes = [1u8; 32];
        let scalar = from_bytes_to_scalar(&scalar_bytes).unwrap();

        // Convert to public key
        let pubkey_result = scalar_to_pubkey(&scalar);
        assert!(pubkey_result.is_ok());

        // Verify the public key matches expected value for this scalar
        let pubkey = pubkey_result.unwrap();
        let point = ProjectivePoint::GENERATOR * scalar;
        let expected_pubkey = PublicKey::from_affine(AffinePoint::from(point)).unwrap();
        assert_eq!(
            pubkey.to_sec1_bytes().as_ref(),
            expected_pubkey.to_sec1_bytes().as_ref()
        );
    }

    #[test]
    fn test_polynomial_evaluation() {
        // Create a polynomial with known coefficients
        // p(x) = 5 + 3x + 2x² (coefficients [5, 3, 2])
        let five = Scalar::from(5u64);
        let three = Scalar::from(3u64);
        let two = Scalar::from(2u64);

        let polynomial = Polynomial {
            coefficients: vec![five, three, two],
            proofs: vec![], // Proofs not needed for this test
        };

        // Test polynomial evaluation at x=2: p(2) = 5 + 3*2 + 2*2² = 5 + 6 + 8 = 19
        let x = Scalar::from(2u64);
        let result = polynomial.evaluate(&x);
        assert_eq!(result, Scalar::from(19u64));

        // Test polynomial evaluation at x=0: p(0) = 5
        let x = Scalar::ZERO;
        let result = polynomial.evaluate(&x);
        assert_eq!(result, five);

        // Test polynomial evaluation at x=1: p(1) = 5 + 3*1 + 2*1² = 10
        let x = Scalar::ONE;
        let result = polynomial.evaluate(&x);
        assert_eq!(result, Scalar::from(10u64));
    }

    #[test]
    fn test_generate_polynomial() {
        // Create a test secret
        let secret_bytes = [1u8; 32];
        let secret = from_bytes_to_scalar(&secret_bytes).unwrap();

        // Generate a polynomial of threshold 3 (degree 2)
        let threshold = 3;
        let poly_result = generate_polynomial_for_secret_sharing(&secret, threshold);

        assert!(poly_result.is_ok());
        let poly = poly_result.unwrap();

        // Check polynomial properties
        assert_eq!(poly.coefficients.len(), threshold);
        assert_eq!(poly.coefficients[0], secret); // Constant term should be the secret
        assert_eq!(poly.proofs.len(), threshold);

        // Verify each proof corresponds to its coefficient
        for (i, coeff) in poly.coefficients.iter().enumerate() {
            let generated_proof = scalar_to_pubkey(coeff).unwrap();
            assert_eq!(&poly.proofs[i], &generated_proof);
        }
    }

    #[test]
    fn test_split_secret_basic() {
        // Create a secret to share
        let secret_bytes = [1u8; 32];
        let secret = from_bytes_to_scalar(&secret_bytes).unwrap();

        // Split the secret: threshold=3, shares=5
        let threshold = 3;
        let num_shares = 5;
        let shares_result = split_secret_with_proofs(&secret, threshold, num_shares);

        assert!(shares_result.is_ok());
        let shares = shares_result.unwrap();

        // Verify share properties
        assert_eq!(shares.len(), num_shares);
        for (i, share) in shares.iter().enumerate() {
            // Index should match share number (1-based)
            assert_eq!(share.secret_share.index, Scalar::from((i + 1) as u64));
            assert_eq!(share.secret_share.threshold, threshold);
            assert_eq!(share.proofs.len(), threshold); // t coefficients in polynomial

            // First coefficient proof should match the public key of the secret
            let secret_pubkey = scalar_to_pubkey(&secret).unwrap();
            assert_eq!(&share.proofs[0], &secret_pubkey);
        }
    }

    #[test]
    fn test_split_secret_invalid_params() {
        let secret_bytes = [1u8; 32];
        let secret = from_bytes_to_scalar(&secret_bytes).unwrap();

        // Test threshold too low
        let result = split_secret_with_proofs(&secret, 0, 5);
        assert!(result.is_err());
        match result {
            Err(SignerError::SecretSharingError(msg)) => {
                assert!(msg.contains("Threshold (0) must be greater than 0 and less than or equal to the number of shares (5)"));
            }
            _ => panic!("Expected SecretSharingError"),
        }

        // Test threshold > number of shares
        let result = split_secret_with_proofs(&secret, 6, 5);
        assert!(result.is_err());
        match result {
            Err(SignerError::SecretSharingError(msg)) => {
                assert!(msg.contains(
                    "Threshold (6) must be greater than 0 and less than or equal to the number of shares (5)"
                ));
            }
            _ => panic!("Expected SecretSharingError"),
        }
    }

    #[test]
    fn test_secret_reconstruction() {
        // This is a simplified implementation of Lagrange interpolation to reconstruct the secret
        // In a real implementation, this would be a separate function in the module
        let reconstruct_secret = |shares: &[VerifiableSecretShare]| -> Result<Scalar, SignerError> {
            if shares.is_empty() {
                return Err(SignerError::SecretSharingError(
                    "No shares provided".to_string(),
                ));
            }

            let threshold = shares[0].secret_share.threshold;
            if shares.len() < threshold {
                return Err(SignerError::SecretSharingError(format!(
                    "Not enough shares. Need {}, got {}",
                    threshold,
                    shares.len()
                )));
            }

            // Lagrange interpolation to find the constant term (the secret)
            let mut result = Scalar::ZERO;

            for i in 0..threshold {
                let share = &shares[i].secret_share;
                let xi = share.index;
                let yi = share.share;

                // Calculate the Lagrange basis polynomial for this point
                let mut numerator = Scalar::ONE;
                let mut denominator = Scalar::ONE;

                for (j, share) in shares.iter().enumerate().take(threshold) {
                    if i == j {
                        continue;
                    }

                    let xj = share.secret_share.index;
                    numerator *= xj;
                    denominator *= xj - xi;
                }

                // Calculate the term yi * L_i(0) and add it to the result
                let lagrange_term = yi * numerator * denominator.invert().unwrap_or(Scalar::ZERO);
                result += lagrange_term;
            }

            Ok(result)
        };

        // Create a secret to share
        let secret_bytes = [42u8; 32];
        let secret = from_bytes_to_scalar(&secret_bytes).unwrap();

        // Split the secret: threshold=3, shares=5
        let threshold = 3;
        let num_shares = 5;
        let all_shares = split_secret_with_proofs(&secret, threshold, num_shares).unwrap();

        // Try to reconstruct with exactly the threshold number of shares
        let subset_shares = &all_shares[0..threshold];
        let reconstructed = reconstruct_secret(subset_shares).unwrap();
        assert_eq!(
            reconstructed, secret,
            "Failed to reconstruct secret with threshold shares"
        );

        // Try to reconstruct with more than the threshold number of shares
        let subset_shares = &all_shares[1..threshold + 2]; // Different selection
        let reconstructed = reconstruct_secret(subset_shares).unwrap();
        assert_eq!(
            reconstructed, secret,
            "Failed to reconstruct secret with more than threshold shares"
        );

        // Try to reconstruct with less than the threshold number of shares (should fail)
        let subset_shares = &all_shares[0..threshold - 1];
        let result = reconstruct_secret(subset_shares);
        assert!(result.is_err());
    }
}
