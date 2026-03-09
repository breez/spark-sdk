import com.google.gson.GsonBuilder
import com.google.gson.JsonElement
import com.google.gson.JsonNull
import com.google.gson.JsonObject
import com.google.gson.JsonArray
import com.google.gson.JsonPrimitive
import com.ionspin.kotlin.bignum.integer.BigInteger

private val gson = GsonBuilder().setPrettyPrinting().serializeNulls().create()

/**
 * Known sealed class / enum prefixes used to extract variant type names.
 * For example, SdkEvent.Synced -> type: "Synced".
 */
private val variantPrefixes = listOf(
    "SendPaymentOptions", "SendPaymentMethod",
    "ReceivePaymentMethod", "PaymentDetailsFilter", "PaymentDetails",
    "LnurlCallbackStatus", "OnchainConfirmationSpeed",
    "ConversionType", "TokenTransactionType",
    "SparkHtlcStatus", "PaymentStatus", "PaymentType",
    "ServiceStatus", "SdkEvent", "InputType",
    "AssetFilter", "FeePolicy", "MaxFee", "Fee",
)

/**
 * Converts a camelCase or PascalCase string to snake_case.
 */
fun toSnakeCase(s: String): String {
    val result = StringBuilder()
    for ((i, ch) in s.withIndex()) {
        if (ch.isUpperCase()) {
            if (i > 0) result.append('_')
            result.append(ch.lowercaseChar())
        } else {
            result.append(ch)
        }
    }
    return result.toString()
}

/**
 * Extracts the variant name from a class name. For example:
 * "SdkEvent$Synced" -> "Synced", "SdkEvent.Synced" -> "Synced"
 */
private fun extractVariantName(className: String): String? {
    // Kotlin sealed classes use $ or . separator
    val simpleName = className.substringAfterLast('$').substringAfterLast('.')

    // Also check against the simple outer class name
    val outerName = className.substringBeforeLast('$', "").substringAfterLast('.')

    for (prefix in variantPrefixes) {
        if (outerName == prefix) {
            return simpleName
        }
        // Handle flat enum-like names: "PaymentStatusCompleted" -> "Completed"
        if (simpleName.startsWith(prefix) && simpleName.length > prefix.length) {
            return simpleName.substring(prefix.length)
        }
    }

    return null
}

/**
 * Recursively converts any object to a Gson JsonElement for serialization.
 * Handles Kotlin sealed classes, data classes, enums, BigInteger, collections, maps, etc.
 */
fun objToJson(value: Any?): JsonElement {
    if (value == null) return JsonNull.INSTANCE

    return when (value) {
        is String -> JsonPrimitive(value)
        is Boolean -> JsonPrimitive(value)
        is Number -> JsonPrimitive(value)
        is UByte -> JsonPrimitive(value.toInt())
        is UShort -> JsonPrimitive(value.toInt())
        is UInt -> JsonPrimitive(value.toLong())
        is ULong -> JsonPrimitive(value.toLong())
        is BigInteger -> JsonPrimitive(value.toString())
        is java.math.BigInteger -> JsonPrimitive(value.toString())

        is ByteArray -> {
            // Encode byte arrays as hex strings
            JsonPrimitive(value.joinToString("") { "%02x".format(it) })
        }

        is Enum<*> -> JsonPrimitive(value.name)

        is List<*> -> {
            val arr = JsonArray()
            for (item in value) {
                arr.add(objToJson(item))
            }
            arr
        }

        is Map<*, *> -> {
            val obj = JsonObject()
            for ((k, v) in value) {
                obj.add(k.toString(), objToJson(v))
            }
            obj
        }

        else -> {
            val clazz = value::class.java
            val obj = JsonObject()

            // Detect sealed class variants
            val className = clazz.name
            val variantName = extractVariantName(className)
            if (variantName != null) {
                obj.addProperty("type", variantName)
            }

            // Enumerate declared fields (including private ones from data/sealed classes)
            val fields = clazz.declaredFields
            for (field in fields) {
                // Skip synthetic fields, companion references, etc.
                if (field.isSynthetic) continue
                if (field.name == "Companion") continue
                if (field.name.startsWith("\$")) continue

                field.isAccessible = true
                val fieldValue = field.get(value)
                val fieldName = toSnakeCase(field.name)
                obj.add(fieldName, objToJson(fieldValue))
            }

            // If we only have "type" and nothing else, try using toString for simple wrappers
            if (obj.size() == 0) {
                return JsonPrimitive(value.toString())
            }

            obj
        }
    }
}

/**
 * Serializes any SDK object to a pretty-printed JSON string.
 */
fun serialize(value: Any?): String {
    return gson.toJson(objToJson(value))
}

/**
 * Prints a serialized value to stdout.
 */
fun printValue(value: Any?) {
    println(serialize(value))
}
