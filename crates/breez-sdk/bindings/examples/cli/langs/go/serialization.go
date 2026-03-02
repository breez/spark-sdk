package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"reflect"
	"strings"
)

// objToMap recursively converts a Go SDK object into a JSON-serializable
// map[string]interface{} using reflection. This handles structs, interfaces
// (tagged unions), slices, maps, *big.Int, and byte slices.
func objToMap(v interface{}) interface{} {
	if v == nil {
		return nil
	}

	val := reflect.ValueOf(v)

	// Dereference pointers
	for val.Kind() == reflect.Ptr {
		if val.IsNil() {
			return nil
		}
		val = val.Elem()
	}

	switch val.Kind() {
	case reflect.String:
		return val.String()
	case reflect.Bool:
		return val.Bool()
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return val.Int()
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64:
		return val.Uint()
	case reflect.Float32, reflect.Float64:
		return val.Float()

	case reflect.Slice:
		// []byte → hex string
		if val.Type().Elem().Kind() == reflect.Uint8 {
			return hex.EncodeToString(val.Bytes())
		}
		result := make([]interface{}, val.Len())
		for i := 0; i < val.Len(); i++ {
			result[i] = objToMap(val.Index(i).Interface())
		}
		return result

	case reflect.Map:
		result := make(map[string]interface{})
		for _, key := range val.MapKeys() {
			result[fmt.Sprintf("%v", key.Interface())] = objToMap(val.MapIndex(key).Interface())
		}
		return result

	case reflect.Struct:
		// Special case: *big.Int
		if bigVal, ok := val.Interface().(big.Int); ok {
			return bigVal.String()
		}

		result := make(map[string]interface{})
		t := val.Type()

		// For tagged union variants, add a "type" field derived from the struct name.
		// E.g. "SdkEventSynced" → strip common prefixes to get "Synced".
		typeName := t.Name()
		if isVariantType(typeName) {
			result["type"] = extractVariantName(typeName)
		}

		for i := 0; i < t.NumField(); i++ {
			field := t.Field(i)
			if !field.IsExported() {
				continue
			}
			fieldVal := val.Field(i)

			// Handle interface fields (tagged unions)
			if field.Type.Kind() == reflect.Interface && !fieldVal.IsNil() {
				result[toSnakeCase(field.Name)] = objToMap(fieldVal.Interface())
				continue
			}

			// Handle pointer fields
			if field.Type.Kind() == reflect.Ptr {
				if fieldVal.IsNil() {
					result[toSnakeCase(field.Name)] = nil
					continue
				}
				result[toSnakeCase(field.Name)] = objToMap(fieldVal.Interface())
				continue
			}

			result[toSnakeCase(field.Name)] = objToMap(fieldVal.Interface())
		}
		return result

	case reflect.Interface:
		if val.IsNil() {
			return nil
		}
		return objToMap(val.Interface())
	}

	return fmt.Sprintf("%v", v)
}

// isVariantType checks if a type name looks like a tagged union variant
// (e.g., "SdkEventSynced", "InputTypeBolt11Invoice").
func isVariantType(name string) bool {
	prefixes := []string{
		"SdkEvent", "InputType", "SendPaymentMethod", "SendPaymentOptions",
		"ReceivePaymentMethod", "PaymentDetailsFilter", "PaymentDetails",
		"LnurlCallbackStatus", "MaxFee", "Fee", "ConversionType",
		"AssetFilter", "OnchainConfirmationSpeed", "FeePolicy",
		"PaymentType", "PaymentStatus", "SparkHtlcStatus",
		"TokenTransactionType", "ServiceStatus",
	}
	for _, prefix := range prefixes {
		if strings.HasPrefix(name, prefix) && name != prefix {
			return true
		}
	}
	return false
}

// extractVariantName strips common SDK prefixes to get the variant name.
func extractVariantName(name string) string {
	prefixes := []string{
		"SendPaymentOptions", "SendPaymentMethod",
		"ReceivePaymentMethod", "PaymentDetailsFilter", "PaymentDetails",
		"LnurlCallbackStatus", "OnchainConfirmationSpeed",
		"ConversionType", "TokenTransactionType",
		"SparkHtlcStatus", "PaymentStatus", "PaymentType",
		"ServiceStatus", "SdkEvent", "InputType",
		"AssetFilter", "FeePolicy", "MaxFee", "Fee",
	}
	for _, prefix := range prefixes {
		if strings.HasPrefix(name, prefix) && len(name) > len(prefix) {
			return name[len(prefix):]
		}
	}
	return name
}

// toSnakeCase converts a PascalCase or camelCase string to snake_case.
func toSnakeCase(s string) string {
	var result strings.Builder
	for i, r := range s {
		if r >= 'A' && r <= 'Z' {
			if i > 0 {
				result.WriteByte('_')
			}
			result.WriteRune(r + 32) // lowercase
		} else {
			result.WriteRune(r)
		}
	}
	return result.String()
}

// serialize converts a Go SDK object to a pretty-printed JSON string.
func serialize(v interface{}) string {
	data, err := json.MarshalIndent(objToMap(v), "", "  ")
	if err != nil {
		return fmt.Sprintf("%v", v)
	}
	return string(data)
}

// printValue prints a Go SDK object as pretty JSON to stdout.
func printValue(v interface{}) {
	fmt.Println(serialize(v))
}
