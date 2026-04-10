// Ambient type declarations for React Native globals provided by Hermes and polyfills.

// Provided by react-native-get-random-values
declare const crypto: {
  getRandomValues<T extends ArrayBufferView>(array: T): T;
};

// Available in Hermes (RN 0.79+)
declare class TextEncoder {
  encode(input?: string): Uint8Array;
}

declare class TextDecoder {
  decode(input?: ArrayBuffer | ArrayBufferView): string;
}

// Base64 encoding/decoding (available in Hermes)
declare function atob(data: string): string;
declare function btoa(data: string): string;
