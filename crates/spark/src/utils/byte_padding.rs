pub trait BytePadding: Sized {
    fn to_unpadded_be_bytes(&self) -> Vec<u8>;
    fn to_unpadded_le_bytes(&self) -> Vec<u8>;

    fn from_unpadded_be_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>>;
    fn from_unpadded_le_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>>;
}

impl BytePadding for u128 {
    fn to_unpadded_be_bytes(&self) -> Vec<u8> {
        if *self == 0 {
            return vec![0];
        }
        self.to_be_bytes()
            .into_iter()
            .skip_while(|&b| b == 0)
            .collect()
    }

    fn to_unpadded_le_bytes(&self) -> Vec<u8> {
        if *self == 0 {
            return vec![0];
        }
        let mut bytes = self.to_le_bytes().to_vec();
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        bytes
    }

    fn from_unpadded_be_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if bytes.len() > 16 {
            return Err("Input bytes exceed u128 size".into());
        }
        Ok(bytes
            .iter()
            .fold(0u128, |acc, &byte| (acc << 8) | u128::from(byte)))
    }

    fn from_unpadded_le_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if bytes.len() > 16 {
            return Err("Input bytes exceed u128 size".into());
        }
        Ok(bytes
            .iter()
            .rev()
            .fold(0u128, |acc, &byte| (acc << 8) | u128::from(byte)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_unpadded_be_bytes() {
        assert_eq!(100u128.to_unpadded_be_bytes(), vec![100]);
        assert_eq!(256u128.to_unpadded_be_bytes(), vec![1, 0]);
        assert_eq!(255u128.to_unpadded_be_bytes(), vec![255]);
        assert_eq!(65535u128.to_unpadded_be_bytes(), vec![255, 255]);
        assert_eq!(16909060u128.to_unpadded_be_bytes(), vec![1, 2, 3, 4]);
        assert_eq!(0u128.to_unpadded_be_bytes(), vec![0]);
        assert_eq!(
            18446744073709551615u128.to_unpadded_be_bytes(),
            vec![255, 255, 255, 255, 255, 255, 255, 255]
        );
        assert_eq!(
            340282366920938463463374607431768211455u128.to_unpadded_be_bytes(),
            vec![
                255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255
            ]
        );
    }

    #[test]
    fn test_to_unpadded_le_bytes() {
        assert_eq!(100u128.to_unpadded_le_bytes(), vec![100]);
        assert_eq!(256u128.to_unpadded_le_bytes(), vec![0, 1]);
        assert_eq!(255u128.to_unpadded_le_bytes(), vec![255]);
        assert_eq!(65535u128.to_unpadded_le_bytes(), vec![255, 255]);
        assert_eq!(16909060u128.to_unpadded_le_bytes(), vec![4, 3, 2, 1]);
        assert_eq!(0u128.to_unpadded_le_bytes(), vec![0]);
        assert_eq!(
            18446744073709551615u128.to_unpadded_le_bytes(),
            vec![255, 255, 255, 255, 255, 255, 255, 255]
        );
        assert_eq!(
            340282366920938463463374607431768211455u128.to_unpadded_le_bytes(),
            vec![
                255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255
            ]
        );
    }

    #[test]
    fn test_from_padded_be_bytes() {
        assert_eq!(u128::from_unpadded_be_bytes(&[100]).unwrap(), 100);
        assert_eq!(u128::from_unpadded_be_bytes(&[1, 0]).unwrap(), 256);
        assert_eq!(u128::from_unpadded_be_bytes(&[255]).unwrap(), 255);
        assert_eq!(u128::from_unpadded_be_bytes(&[0, 0, 1, 0]).unwrap(), 256);
        assert_eq!(u128::from_unpadded_be_bytes(&[255, 255]).unwrap(), 65535);
        assert_eq!(
            u128::from_unpadded_be_bytes(&[1, 2, 3, 4]).unwrap(),
            16909060
        );
        assert_eq!(u128::from_unpadded_be_bytes(&[]).unwrap(), 0);
        assert_eq!(u128::from_unpadded_be_bytes(&[0]).unwrap(), 0);
        assert_eq!(
            u128::from_unpadded_be_bytes(&[255, 255, 255, 255, 255, 255, 255, 255]).unwrap(),
            18446744073709551615
        );
        assert_eq!(
            u128::from_unpadded_be_bytes(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 100])
                .unwrap(),
            100
        );
        assert!(u128::from_unpadded_be_bytes(&[1; 17]).is_err());
    }

    #[test]
    fn test_from_padded_le_bytes() {
        assert_eq!(u128::from_unpadded_le_bytes(&[100]).unwrap(), 100);
        assert_eq!(u128::from_unpadded_le_bytes(&[1, 0]).unwrap(), 1);
        assert_eq!(u128::from_unpadded_le_bytes(&[0, 1]).unwrap(), 256);
        assert_eq!(u128::from_unpadded_le_bytes(&[255]).unwrap(), 255);
        assert_eq!(u128::from_unpadded_le_bytes(&[0, 1, 0, 0]).unwrap(), 256);
        assert_eq!(u128::from_unpadded_le_bytes(&[255, 255]).unwrap(), 65535);
        assert_eq!(
            u128::from_unpadded_le_bytes(&[4, 3, 2, 1]).unwrap(),
            16909060
        );
        assert_eq!(u128::from_unpadded_le_bytes(&[]).unwrap(), 0);
        assert_eq!(u128::from_unpadded_le_bytes(&[0]).unwrap(), 0);
        assert_eq!(
            u128::from_unpadded_le_bytes(&[255, 255, 255, 255, 255, 255, 255, 255]).unwrap(),
            18446744073709551615
        );
        assert_eq!(
            u128::from_unpadded_le_bytes(&[100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
                .unwrap(),
            100
        );
        assert!(u128::from_unpadded_le_bytes(&[1; 17]).is_err());
    }
}
