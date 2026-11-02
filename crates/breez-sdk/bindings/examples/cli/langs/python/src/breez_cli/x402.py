import argparse
import asyncio
import base64
import http.client
import json
from collections import namedtuple
from urllib.parse import urlsplit, urlunsplit

from breez_sdk_spark import (
    PayX402Request,
    X402Header,
    X402PaymentAmount,
    X402PaymentStatus,
    X402Request,
)

# x402 subcommand names (used for REPL completion)
X402_COMMAND_NAMES = [
    "x402 fetch",
]

PAYMENT_REQUIRED_HEADER = "payment-required"
PAYMENT_SIGNATURE_HEADER = "payment-signature"
PAYMENT_RESPONSE_HEADER = "payment-response"
# Times to pay the same challenge again while its payment is pending. Fetching
# again instead would bring a new challenge with a new invoice to pay.
PENDING_RETRIES = 3
# Times to send the paid request again while the server answers that the
# payment may still settle.
SETTLE_RETRIES = 5
SETTLE_RETRY_DELAY_SECS = 2
REQUEST_TIMEOUT_SECS = 60
REDIRECT_STATUSES = (301, 302, 303, 307, 308)
DEFAULT_PORTS = {"http": 80, "https": 443}

# Response headers have lowercased names.
HttpResponse = namedtuple("HttpResponse", ["status", "headers", "body"])


def _parser(name, description=""):
    return argparse.ArgumentParser(prog=f"x402 {name}", description=description)


# --- fetch ---

def _build_fetch_parser():
    p = _parser("fetch", "Fetch a URL, paying its x402 Lightning challenge if the server asks for one")
    p.add_argument("url", help="The URL to fetch")
    p.add_argument("--max-amount-sats", type=int, required=True,
                   help="The most to spend in sats, routing fee included")
    p.add_argument("-X", "--method", default="GET", help="The HTTP method: GET or POST")
    p.add_argument("-d", "--body", default=None, help="The request body")
    p.add_argument("-H", "--header", dest="headers", action="append", default=None,
                   help='A request header as "Name: value". Can be repeated')
    return p

async def _handle_fetch(sdk, args):
    method = args.method
    body = args.body
    if method == "GET" and body is not None:
        raise ValueError("a GET request has no body: use -X POST to send one")
    # The server hashes the headers it binds, so send every header
    # explicitly: anything the HTTP client added on its own would be
    # missing from the hash computed here.
    headers = _parse_headers(args.headers or [])
    headers.setdefault("accept", "*/*")
    url = _as_sent(args.url)

    response = await _send(method, url, headers, body)
    if response.status != 402:
        _print_response(response)
        return
    payment_required = response.headers.get(PAYMENT_REQUIRED_HEADER)
    if payment_required is None:
        raise ValueError("402 response has no PAYMENT-REQUIRED header")

    x402 = sdk.get_x402()
    request = PayX402Request(
        payment_required=payment_required,
        request=X402Request.HTTP(
            method=method,
            url=url,
            headers=[X402Header(name=name, value=value) for name, value in headers.items()],
            body=body.encode() if body is not None else None,
        ),
        max_amount=X402PaymentAmount.LIGHTNING(amount_sats=args.max_amount_sats),
        conversion_options=None,
        completion_timeout_secs=None,
    )
    paid = await x402.pay(request=request)
    for _ in range(PENDING_RETRIES):
        if paid.status == X402PaymentStatus.PAID:
            break
        print("Payment is pending, waiting for it to complete...")
        paid = await x402.pay(request=request)
    # The payment signature proves the payment, so it isn't printed.
    print(
        f"x402 payment {paid.payment.id} is {paid.status.name}: "
        f"{paid.payment.amount} sats, fee {paid.payment.fees} sats"
    )
    if paid.payment_signature is None:
        print("Payment is still pending. Check its status with list-payments.")
        return

    headers[PAYMENT_SIGNATURE_HEADER] = paid.payment_signature
    response = await _send(method, url, headers, body)
    for _ in range(SETTLE_RETRIES):
        if not _settlement_pending(response):
            break
        print("Payment is not settled yet, sending it again...")
        await asyncio.sleep(SETTLE_RETRY_DELAY_SECS)
        response = await _send(method, url, headers, body)
    _print_response(response)


def _parse_headers(lines):
    headers = {}
    for line in lines:
        name, sep, value = line.partition(":")
        if not sep:
            raise ValueError(f'header "{line}" is not "Name: value"')
        headers[name.strip().lower()] = value.strip()
    return headers


async def _send(method, url, headers, body):
    if method == "GET":
        data = None
    elif method == "POST":
        data = body.encode() if body is not None else b""
    else:
        raise ValueError(f"unsupported method {method}: use GET or POST")
    response = await asyncio.to_thread(_send_once, method, url, headers, data)
    if response.status in REDIRECT_STATUSES:
        location = response.headers.get("location", "<no location>")
        raise ValueError(
            f"endpoint returned a {response.status} redirect to {location}, "
            "which this client does not follow"
        )
    return response


# The URL as http.client sends it: lowercase scheme and host, no default port,
# "/" for no path, and no fragment.
def _as_sent(url):
    parts = urlsplit(url)
    host = parts.hostname or ""
    netloc = f"[{host}]" if ":" in host else host
    if parts.port not in (None, DEFAULT_PORTS.get(parts.scheme)):
        netloc += f":{parts.port}"
    userinfo, at, _ = parts.netloc.rpartition("@")
    return urlunsplit((parts.scheme, userinfo + at + netloc, parts.path or "/", parts.query, ""))


def _send_once(method, url, headers, data):
    parts = urlsplit(url)
    connection_classes = {"http": http.client.HTTPConnection, "https": http.client.HTTPSConnection}
    if parts.scheme not in connection_classes or not parts.hostname:
        raise ValueError(f"unsupported URL {url}: use an http or https URL")
    path = parts.path or "/"
    if parts.query:
        path += f"?{parts.query}"
    connection = connection_classes[parts.scheme](
        parts.hostname, parts.port, timeout=REQUEST_TIMEOUT_SECS
    )
    try:
        # putrequest, unlike request, can leave out the implicit Accept-Encoding.
        connection.putrequest(
            method, path, skip_host="host" in headers, skip_accept_encoding=True
        )
        for name, value in headers.items():
            connection.putheader(name, value)
        if data is not None and "content-length" not in headers:
            connection.putheader("content-length", str(len(data)))
        connection.endheaders(data)
        return _read_response(connection.getresponse())
    finally:
        connection.close()


def _read_response(response):
    headers = {name.lower(): value for name, value in response.headers.items()}
    body = response.read().decode("utf-8", errors="replace")
    return HttpResponse(status=response.status, headers=headers, body=body)


# Whether the server answered that the payment may still settle: a 402 with no
# new challenge and the core x402 specification's non-final reason.
def _settlement_pending(response):
    settlement = response.headers.get(PAYMENT_RESPONSE_HEADER)
    if response.status != 402 or PAYMENT_REQUIRED_HEADER in response.headers or settlement is None:
        return False
    try:
        decoded = json.loads(base64.b64decode(settlement, validate=True).decode("utf-8"))
    except ValueError:
        return False
    return isinstance(decoded, dict) and decoded.get("errorReason") == "settlement_pending"


def _print_response(response):
    print(f"HTTP {response.status}")
    settlement = response.headers.get(PAYMENT_RESPONSE_HEADER)
    if settlement is not None:
        try:
            decoded = base64.b64decode(settlement, validate=True)
            print(f"PAYMENT-RESPONSE: {decoded.decode('utf-8', errors='replace')}")
        except ValueError:
            pass
    print(response.body)


# ---------------------------------------------------------------------------
# Registry and dispatch
# ---------------------------------------------------------------------------

def _build_x402_registry():
    return {
        "fetch": (_build_fetch_parser(), _handle_fetch),
    }


_REGISTRY = None

def _get_registry():
    global _REGISTRY
    if _REGISTRY is None:
        _REGISTRY = _build_x402_registry()
    return _REGISTRY


async def dispatch_x402_command(args, sdk):
    """Dispatch an x402 subcommand given the args after 'x402'."""
    registry = _get_registry()

    if not args or args[0] == "help":
        print("\nx402 subcommands:\n")
        for name, (parser, _) in sorted(registry.items()):
            desc = parser.description or ""
            print(f"  x402 {name:34s} {desc}")
        print()
        return

    sub_name = args[0]
    sub_args = args[1:]

    if sub_name not in registry:
        print(f"Unknown x402 subcommand: {sub_name}. Use 'x402 help' for available commands.")
        return

    parser, handler = registry[sub_name]
    try:
        parsed = parser.parse_args(sub_args)
    except SystemExit:
        return

    await handler(sdk, parsed)
