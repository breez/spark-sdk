import json


def obj_to_dict(obj):
    """Recursively convert a UniFFI-generated object to a JSON-serializable dict."""
    if obj is None:
        return None
    if isinstance(obj, (str, int, float, bool)):
        return obj
    if isinstance(obj, (list, tuple)):
        return [obj_to_dict(item) for item in obj]
    if isinstance(obj, dict):
        return {str(k): obj_to_dict(v) for k, v in obj.items()}
    if isinstance(obj, bytes):
        return obj.hex()

    cls = type(obj)
    cls_name = cls.__qualname__

    result = {}

    # For enum variants (inner classes like SdkEvent.SYNCED), include variant type
    if "." in cls_name:
        _parent, variant = cls_name.rsplit(".", 1)
        result["type"] = variant.lower()

    for attr_name in sorted(dir(obj)):
        if attr_name.startswith("_"):
            continue
        try:
            val = getattr(obj, attr_name)
        except Exception:
            continue
        if callable(val):
            continue
        result[attr_name] = obj_to_dict(val)

    return result if result else str(obj)


def serialize(obj) -> str:
    """Serialize a UniFFI object to a pretty-printed JSON string."""
    return json.dumps(obj_to_dict(obj), indent=2, default=str)


def print_value(obj):
    """Print a UniFFI object as pretty JSON."""
    print(serialize(obj))
