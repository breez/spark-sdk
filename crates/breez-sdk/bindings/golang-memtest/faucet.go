package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"sync"
	"time"
)

// Faucet is a GraphQL client for the regtest faucet.
type Faucet struct {
	url      string
	username string
	password string
	client   *http.Client

	// Rate limiting semaphore
	sem chan struct{}
}

// GraphQL request/response types
type graphQLRequest struct {
	OperationName string      `json:"operationName"`
	Variables     interface{} `json:"variables"`
	Query         string      `json:"query"`
}

type faucetVariables struct {
	AmountSats uint64 `json:"amount_sats"`
	Address    string `json:"address"`
}

type graphQLResponse struct {
	Data   *responseData  `json:"data,omitempty"`
	Errors []graphQLError `json:"errors,omitempty"`
}

type responseData struct {
	RequestRegtestFunds requestRegtestFunds `json:"request_regtest_funds"`
}

type requestRegtestFunds struct {
	TransactionHash string `json:"transaction_hash"`
}

type graphQLError struct {
	Message string `json:"message"`
}

// NewFaucet creates a new faucet client.
func NewFaucet(url, username, password string) *Faucet {
	return &Faucet{
		url:      url,
		username: username,
		password: password,
		client: &http.Client{
			Timeout: 30 * time.Second,
		},
		sem: make(chan struct{}, 2), // Max 2 concurrent requests
	}
}

// FundAddress requests funds from the faucet.
func (f *Faucet) FundAddress(ctx context.Context, address string, amountSats uint64) (string, error) {
	// Acquire semaphore
	select {
	case f.sem <- struct{}{}:
		defer func() { <-f.sem }()
	case <-ctx.Done():
		return "", ctx.Err()
	}

	reqBody := graphQLRequest{
		OperationName: "RequestRegtestFunds",
		Variables: faucetVariables{
			AmountSats: amountSats,
			Address:    address,
		},
		Query: `mutation RequestRegtestFunds($address: String!, $amount_sats: Long!) {
			request_regtest_funds(input: {address: $address, amount_sats: $amount_sats}) {
				transaction_hash
			}
		}`,
	}

	body, err := json.Marshal(reqBody)
	if err != nil {
		return "", fmt.Errorf("failed to marshal request: %w", err)
	}

	req, err := http.NewRequestWithContext(ctx, "POST", f.url, bytes.NewReader(body))
	if err != nil {
		return "", fmt.Errorf("failed to create request: %w", err)
	}

	req.Header.Set("Content-Type", "application/json")

	// Add basic auth if credentials provided
	if f.username != "" && f.password != "" {
		req.SetBasicAuth(f.username, f.password)
	}

	resp, err := f.client.Do(req)
	if err != nil {
		return "", fmt.Errorf("request failed: %w", err)
	}
	defer resp.Body.Close()

	respBody, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", fmt.Errorf("failed to read response: %w", err)
	}

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("faucet returned status %d: %s", resp.StatusCode, string(respBody))
	}

	var graphResp graphQLResponse
	if err := json.Unmarshal(respBody, &graphResp); err != nil {
		return "", fmt.Errorf("failed to unmarshal response: %w", err)
	}

	if len(graphResp.Errors) > 0 {
		return "", fmt.Errorf("GraphQL error: %s", graphResp.Errors[0].Message)
	}

	if graphResp.Data == nil {
		return "", fmt.Errorf("no data in response")
	}

	return graphResp.Data.RequestRegtestFunds.TransactionHash, nil
}

// FundAddressWithRetry retries funding with exponential backoff.
func (f *Faucet) FundAddressWithRetry(ctx context.Context, address string, amountSats uint64, maxRetries int) (string, error) {
	var lastErr error
	for i := 0; i < maxRetries; i++ {
		txHash, err := f.FundAddress(ctx, address, amountSats)
		if err == nil {
			return txHash, nil
		}
		lastErr = err

		// Exponential backoff
		backoff := time.Duration(1<<uint(i)) * time.Second
		if backoff > 30*time.Second {
			backoff = 30 * time.Second
		}

		select {
		case <-time.After(backoff):
		case <-ctx.Done():
			return "", ctx.Err()
		}
	}
	return "", fmt.Errorf("failed after %d retries: %w", maxRetries, lastErr)
}

// FaucetPool manages funding for multiple SDK instances.
type FaucetPool struct {
	faucet *Faucet
	mu     sync.Mutex
	funded map[string]bool // Track funded addresses
}

// NewFaucetPool creates a new faucet pool.
func NewFaucetPool(faucet *Faucet) *FaucetPool {
	return &FaucetPool{
		faucet: faucet,
		funded: make(map[string]bool),
	}
}

// EnsureFunded ensures an address has been funded at least once.
func (p *FaucetPool) EnsureFunded(ctx context.Context, address string, amountSats uint64) error {
	p.mu.Lock()
	if p.funded[address] {
		p.mu.Unlock()
		return nil
	}
	p.mu.Unlock()

	txHash, err := p.faucet.FundAddressWithRetry(ctx, address, amountSats, 3)
	if err != nil {
		return err
	}

	fmt.Printf("Funded %s with %d sats (tx: %s)\n", truncateAddress(address), amountSats, txHash)

	p.mu.Lock()
	p.funded[address] = true
	p.mu.Unlock()

	return nil
}

// truncateAddress shortens an address for display.
func truncateAddress(addr string) string {
	if len(addr) > 16 {
		return addr[:8] + "..." + addr[len(addr)-8:]
	}
	return addr
}
