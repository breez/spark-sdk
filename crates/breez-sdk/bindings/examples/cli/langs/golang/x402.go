package main

import (
	"encoding/base64"
	"flag"
	"fmt"
	"io"
	"net/http"
	"sort"
	"strconv"
	"strings"
	"time"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

const (
	x402PaymentRequiredHeader  = "payment-required"
	x402PaymentSignatureHeader = "payment-signature"
	x402PaymentResponseHeader  = "payment-response"
	// Times to pay the same challenge again while its payment is pending. Fetching
	// again instead would bring a new challenge with a new invoice to pay.
	x402PendingRetries = 3
	// Times to send the paid request again while the server answers 402 without a
	// new challenge, meaning the payment may still settle.
	x402SettleRetries    = 5
	x402SettleRetryDelay = 2 * time.Second
	x402RequestTimeout   = 60 * time.Second
)

// X402Cmd represents a single x402 subcommand.
type X402Cmd struct {
	Name        string
	Description string
	Run         func(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error
}

// X402CommandNames lists all x402 subcommand names (used for REPL completion).
var X402CommandNames = []string{
	"x402 fetch",
}

// BuildX402Registry returns a map of x402 subcommand name to X402Cmd.
func BuildX402Registry() map[string]X402Cmd {
	return map[string]X402Cmd{
		"fetch": {
			Name:        "fetch",
			Description: "Fetch a URL, paying its x402 Lightning challenge if the server asks for one",
			Run:         handleX402Fetch,
		},
	}
}

// DispatchX402Command dispatches an x402 subcommand.
func DispatchX402Command(args []string, sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance) {
	registry := BuildX402Registry()

	if len(args) == 0 || args[0] == "help" {
		fmt.Println("\nx402 subcommands:")
		names := make([]string, 0, len(registry))
		for name := range registry {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			cmd := registry[name]
			fmt.Printf("  x402 %-30s %s\n", name, cmd.Description)
		}
		fmt.Println()
		return
	}

	subName := args[0]
	subArgs := args[1:]

	cmd, ok := registry[subName]
	if !ok {
		fmt.Printf("Unknown x402 subcommand: %s. Use 'x402 help' for available commands.\n", subName)
		return
	}

	if err := cmd.Run(sdk, rl, subArgs); err != nil {
		fmt.Printf("Error: %v\n", err)
	}
}

// ---------------------------------------------------------------------------
// x402 command handlers
// ---------------------------------------------------------------------------

// --- fetch ---

func handleX402Fetch(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("x402 fetch", flag.ContinueOnError)
	maxAmountStr := fs.String("max-amount-sats", "", "The most to spend in sats, routing fee included")
	method := fs.String("X", "GET", "The HTTP method: GET or POST")
	fs.StringVar(method, "method", "GET", "The HTTP method: GET or POST")
	bodyStr := fs.String("d", "", "The request body")
	fs.StringVar(bodyStr, "body", "", "The request body")
	var headerLines repeatedStringFlag
	fs.Var(&headerLines, "H", `A request header as "Name: value". Can be repeated`)
	fs.Var(&headerLines, "header", `A request header as "Name: value". Can be repeated`)
	positional, err := parseInterleavedFlags(fs, args)
	if err != nil {
		return err
	}

	if len(positional) < 1 || *maxAmountStr == "" {
		fmt.Println(`Usage: x402 fetch <url> --max-amount-sats <sats> [-X GET|POST] [-d <body>] [-H "Name: value"]...`)
		return nil
	}
	url := positional[0]
	maxAmountSats, err := strconv.ParseUint(*maxAmountStr, 10, 64)
	if err != nil {
		return fmt.Errorf("invalid max amount: %w", err)
	}
	var body *string
	fs.Visit(func(f *flag.Flag) {
		if f.Name == "d" || f.Name == "body" {
			body = bodyStr
		}
	})

	client := &http.Client{Timeout: x402RequestTimeout}
	// The server hashes the headers it binds, so send every header
	// explicitly: anything the HTTP client added on its own would be
	// missing from the hash computed here.
	headers, err := parseX402Headers(headerLines)
	if err != nil {
		return err
	}
	if _, ok := findX402Header(headers, "accept"); !ok {
		headers = setX402Header(headers, "accept", "*/*")
	}

	response, err := sendX402Request(client, *method, url, headers, body)
	if err != nil {
		return err
	}
	if response.status != http.StatusPaymentRequired {
		printX402Response(response)
		return nil
	}
	paymentRequired, ok := response.header(x402PaymentRequiredHeader)
	if !ok {
		return fmt.Errorf("402 response has no PAYMENT-REQUIRED header")
	}

	var requestBody *[]byte
	if body != nil {
		b := []byte(*body)
		requestBody = &b
	}
	x402 := sdk.GetX402()
	request := breez_sdk_spark.PayX402Request{
		PaymentRequired: paymentRequired,
		Request: breez_sdk_spark.X402RequestHttp{
			Method:  *method,
			Url:     url,
			Headers: append([]breez_sdk_spark.X402Header(nil), headers...),
			Body:    requestBody,
		},
		MaxAmount:             breez_sdk_spark.X402PaymentAmountLightning{AmountSats: maxAmountSats},
		ConversionOptions:     nil,
		CompletionTimeoutSecs: nil,
	}
	paid, err := x402.Pay(request)
	if err = liftError(err); err != nil {
		return err
	}
	for i := 0; i < x402PendingRetries; i++ {
		if paid.Status == breez_sdk_spark.X402PaymentStatusPaid {
			break
		}
		fmt.Println("Payment is pending, waiting for it to complete...")
		paid, err = x402.Pay(request)
		if err = liftError(err); err != nil {
			return err
		}
	}
	printValue(paid)
	if paid.PaymentSignature == nil {
		fmt.Println("Payment is still pending. Check its status with list-payments.")
		return nil
	}

	headers = setX402Header(headers, x402PaymentSignatureHeader, *paid.PaymentSignature)
	response, err = sendX402Request(client, *method, url, headers, body)
	if err != nil {
		return err
	}
	for i := 0; i < x402SettleRetries; i++ {
		if response.status != http.StatusPaymentRequired {
			break
		}
		if _, ok := response.header(x402PaymentRequiredHeader); ok {
			break
		}
		fmt.Println("Payment is not settled yet, sending it again...")
		time.Sleep(x402SettleRetryDelay)
		response, err = sendX402Request(client, *method, url, headers, body)
		if err != nil {
			return err
		}
	}
	printX402Response(response)
	return nil
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// repeatedStringFlag implements flag.Value, keeping each occurrence whole.
type repeatedStringFlag []string

func (s *repeatedStringFlag) String() string {
	return strings.Join(*s, ", ")
}

func (s *repeatedStringFlag) Set(value string) error {
	*s = append(*s, value)
	return nil
}

// parseInterleavedFlags parses flags that may come before or after positional
// arguments, returning the positional arguments in order.
func parseInterleavedFlags(fs *flag.FlagSet, args []string) ([]string, error) {
	var positional []string
	for {
		if err := fs.Parse(args); err != nil {
			return nil, err
		}
		args = fs.Args()
		if len(args) == 0 {
			return positional, nil
		}
		positional = append(positional, args[0])
		args = args[1:]
	}
}

func parseX402Headers(lines []string) ([]breez_sdk_spark.X402Header, error) {
	headers := []breez_sdk_spark.X402Header{}
	for _, line := range lines {
		name, value, ok := strings.Cut(line, ":")
		if !ok {
			return nil, fmt.Errorf("header %q is not \"Name: value\"", line)
		}
		headers = setX402Header(headers, strings.ToLower(strings.TrimSpace(name)), strings.TrimSpace(value))
	}
	return headers, nil
}

func findX402Header(headers []breez_sdk_spark.X402Header, name string) (int, bool) {
	for i, header := range headers {
		if header.Name == name {
			return i, true
		}
	}
	return 0, false
}

func setX402Header(headers []breez_sdk_spark.X402Header, name, value string) []breez_sdk_spark.X402Header {
	if i, ok := findX402Header(headers, name); ok {
		headers[i].Value = value
		return headers
	}
	return append(headers, breez_sdk_spark.X402Header{Name: name, Value: value})
}

type x402Response struct {
	status  int
	headers http.Header
	body    string
}

// header returns the last value of a response header, looked up case-insensitively.
func (r *x402Response) header(name string) (string, bool) {
	values := r.headers.Values(name)
	if len(values) == 0 {
		return "", false
	}
	return values[len(values)-1], true
}

func sendX402Request(
	client *http.Client,
	method, url string,
	headers []breez_sdk_spark.X402Header,
	body *string,
) (*x402Response, error) {
	var requestBody io.Reader
	switch method {
	case "GET":
	case "POST":
		if body != nil {
			requestBody = strings.NewReader(*body)
		}
	default:
		return nil, fmt.Errorf("unsupported method %s: use GET or POST", method)
	}

	req, err := http.NewRequest(method, url, requestBody)
	if err != nil {
		return nil, err
	}
	for _, header := range headers {
		req.Header.Set(header.Name, header.Value)
	}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	// Redirects are followed, so a redirect status here is one that could not be.
	switch resp.StatusCode {
	case 301, 302, 303, 307, 308:
		location := resp.Header.Get("location")
		if location == "" {
			location = "<no location>"
		}
		return nil, fmt.Errorf(
			"endpoint returned a %d redirect to %s, which this client does not follow",
			resp.StatusCode, location,
		)
	}
	return &x402Response{status: resp.StatusCode, headers: resp.Header, body: string(data)}, nil
}

func printX402Response(response *x402Response) {
	fmt.Printf("HTTP %d\n", response.status)
	if settlement, ok := response.header(x402PaymentResponseHeader); ok {
		if decoded, err := base64.StdEncoding.DecodeString(settlement); err == nil {
			fmt.Printf("PAYMENT-RESPONSE: %s\n", decoded)
		}
	}
	fmt.Println(response.body)
}
