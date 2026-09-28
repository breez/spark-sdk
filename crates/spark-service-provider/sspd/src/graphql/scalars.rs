use async_graphql::{InputValueError, InputValueResult, Scalar, ScalarType, Value};

/// A Secp256k1 public key represented as a hex string.
#[derive(Debug, Clone, Default)]
pub struct PublicKey(pub String);

#[Scalar]
impl ScalarType for PublicKey {
    fn parse(value: Value) -> InputValueResult<Self> {
        match &value {
            Value::String(s) => Ok(PublicKey(s.clone())),
            _ => Err(InputValueError::expected_type(value)),
        }
    }

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }
}

/// A 32-byte value represented as a hex string.
#[derive(Debug, Clone, Default)]
pub struct Hash32(pub String);

#[Scalar]
impl ScalarType for Hash32 {
    fn parse(value: Value) -> InputValueResult<Self> {
        match &value {
            Value::String(s) => Ok(Hash32(s.clone())),
            _ => Err(InputValueError::expected_type(value)),
        }
    }

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Long(pub i64);

#[Scalar]
impl ScalarType for Long {
    fn parse(value: Value) -> InputValueResult<Self> {
        match &value {
            Value::Number(n) => {
                if let Some(n) = n.as_i64() {
                    Ok(Long(n))
                } else if let Some(n) = n.as_u64() {
                    Ok(Long(n.cast_signed()))
                } else {
                    Err(InputValueError::expected_type(value))
                }
            }
            Value::String(s) => {
                let n: i64 = s
                    .parse()
                    .map_err(|_| InputValueError::expected_type(value.clone()))?;
                Ok(Long(n))
            }
            _ => Err(InputValueError::expected_type(value)),
        }
    }

    fn to_value(&self) -> Value {
        Value::Number(self.0.into())
    }
}
