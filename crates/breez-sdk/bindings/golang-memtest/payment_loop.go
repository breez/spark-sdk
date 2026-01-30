package main

import (
	"context"
	"fmt"
	"runtime"
	"sync/atomic"
	"time"

	sdk "breez_sdk_spark_go/breez_sdk_spark"
)

// PaymentLoop handles continuous payment exchanges between two SDK instances.
type PaymentLoop struct {
	pair         *SdkPair
	faucet       *FaucetPool
	cfg          *Config
	paymentCount int64
	stopCh       chan struct{}

	// Track payment count at last reconnect to avoid infinite reconnect loop
	lastReconnectAt int64

	// Listener churn manager
	aliceListeners *ListenerManager
	bobListeners   *ListenerManager
}

// NewPaymentLoop creates a new payment loop.
func NewPaymentLoop(pair *SdkPair, faucet *FaucetPool, cfg *Config) *PaymentLoop {
	return &PaymentLoop{
		pair:   pair,
		faucet: faucet,
		cfg:    cfg,
		stopCh: make(chan struct{}),
	}
}

// GetPaymentCount returns the current payment count.
func (p *PaymentLoop) GetPaymentCount() *int64 {
	return &p.paymentCount
}

// GetListenerCount returns a function that returns the total listener count.
func (p *PaymentLoop) GetListenerCount() func() int {
	return func() int {
		count := 2 // Base listeners for Alice and Bob
		if p.aliceListeners != nil {
			count += p.aliceListeners.Count()
		}
		if p.bobListeners != nil {
			count += p.bobListeners.Count()
		}
		return count
	}
}

// FundInitial funds the wallets and waits for funds to be available.
// This should be called before Run() and before the test timer starts.
func (p *PaymentLoop) FundInitial(ctx context.Context) error {
	return p.fundInitial(ctx)
}

// Run starts the payment loop. FundInitial should be called first.
func (p *PaymentLoop) Run(ctx context.Context) error {
	// Initialize listener managers if churn is enabled
	if p.cfg.ListenerChurn {
		p.aliceListeners = NewListenerManager(p.pair.Alice.SDK)
		p.bobListeners = NewListenerManager(p.pair.Bob.SDK)
	}

	ticker := time.NewTicker(p.cfg.PaymentInterval)
	defer ticker.Stop()

	// Alternate payment direction
	aliceToBob := true

	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-p.stopCh:
			return nil
		case <-ticker.C:
			// Check for reconnect cycle
			currentCount := atomic.LoadInt64(&p.paymentCount)
			if p.cfg.ReconnectCycles && currentCount > 0 &&
				currentCount%int64(p.cfg.ReconnectEvery) == 0 &&
				currentCount != p.lastReconnectAt {
				fmt.Println("\n=== Reconnect cycle ===")
				if err := p.reconnectCycle(ctx); err != nil {
					fmt.Printf("Reconnect error: %v\n", err)
				}
				p.lastReconnectAt = currentCount
				continue
			}

			// Perform listener churn if enabled
			if p.cfg.ListenerChurn {
				p.performListenerChurn()
			}

			// Perform frequent sync if enabled
			if p.cfg.FrequentSync {
				p.pair.Alice.SDK.SyncWallet(sdk.SyncWalletRequest{})
				p.pair.Bob.SDK.SyncWallet(sdk.SyncWalletRequest{})
			}

			// Query payment history if enabled
			if p.cfg.PaymentHistoryQueries {
				req := sdk.ListPaymentsRequest{}
				if p.cfg.PaymentHistoryLimit > 0 {
					limit := p.cfg.PaymentHistoryLimit
					req.Limit = &limit
				}
				aliceResp, _ := p.pair.Alice.SDK.ListPayments(req)
				bobResp, _ := p.pair.Bob.SDK.ListPayments(req)

				// Optionally destroy responses to free memory
				if p.cfg.DestroyResponses {
					aliceResp.Destroy()
					bobResp.Destroy()
				}
			}

			// Optionally force GC
			if p.cfg.ForceGC {
				runtime.GC()
			}

			// Check and refund sender if balance is too low (Lightning fees drain funds)
			var sender *SdkInstance
			if aliceToBob {
				sender = p.pair.Alice
			} else {
				sender = p.pair.Bob
			}
			if err := p.checkAndRefundIfNeeded(ctx, sender); err != nil {
				fmt.Printf("Refund error: %v\n", err)
				// Continue anyway, payment may still succeed
			}

			// Execute payment
			var err error
			if aliceToBob {
				err = p.sendPayment(ctx, p.pair.Alice, p.pair.Bob, p.cfg.AmountSats)
			} else {
				err = p.sendPayment(ctx, p.pair.Bob, p.pair.Alice, p.cfg.AmountSats)
			}

			if err != nil {
				fmt.Printf("Payment error: %v\n", err)
			} else {
				atomic.AddInt64(&p.paymentCount, 1)
			}

			aliceToBob = !aliceToBob
		}
	}
}

// Stop stops the payment loop.
func (p *PaymentLoop) Stop() {
	close(p.stopCh)

	// Clean up listener managers
	if p.aliceListeners != nil {
		p.aliceListeners.RemoveAll()
	}
	if p.bobListeners != nil {
		p.bobListeners.RemoveAll()
	}
}

// fundInitial funds both wallets with initial balance and waits for funds to be available.
// Note: Faucet allows max 50k sats per request, so we fund conservatively
// and rely on ping-pong payments to keep funds circulating.
func (p *PaymentLoop) fundInitial(ctx context.Context) error {
	// Fund Alice using Bitcoin address (max 50k sats from faucet)
	aliceBalance, _ := p.pair.Alice.GetBalance()
	if aliceBalance < 10000 {
		if err := p.faucet.EnsureFunded(ctx, p.pair.Alice.BitcoinAddr, 50000); err != nil {
			return fmt.Errorf("failed to fund Alice: %w", err)
		}
	}

	// Fund Bob using Bitcoin address (max 50k sats from faucet)
	bobBalance, _ := p.pair.Bob.GetBalance()
	if bobBalance < 10000 {
		if err := p.faucet.EnsureFunded(ctx, p.pair.Bob.BitcoinAddr, 50000); err != nil {
			return fmt.Errorf("failed to fund Bob: %w", err)
		}
	}

	// Wait for funds to be available (poll until both have sufficient balance)
	fmt.Println("Waiting for funds to be confirmed...")
	minBalance := uint64(10000) // Need at least 10k sats to start
	maxWait := 5 * time.Minute
	pollInterval := 5 * time.Second
	startWait := time.Now()

	for {
		aliceBalance, _ := p.pair.Alice.GetBalance()
		bobBalance, _ := p.pair.Bob.GetBalance()

		if aliceBalance >= minBalance && bobBalance >= minBalance {
			fmt.Printf("Funds confirmed: Alice=%d sats, Bob=%d sats\n", aliceBalance, bobBalance)
			return nil
		}

		if time.Since(startWait) > maxWait {
			return fmt.Errorf("timeout waiting for funds: Alice=%d, Bob=%d", aliceBalance, bobBalance)
		}

		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(pollInterval):
			fmt.Printf("Waiting for funds... Alice=%d sats, Bob=%d sats\n", aliceBalance, bobBalance)
		}
	}
}

// sendSparkPayment sends a Spark payment from sender to receiver.
func (p *PaymentLoop) sendSparkPayment(ctx context.Context, sender, receiver *SdkInstance, amountSats uint64) error {
	sender.mu.Lock()
	senderSDK := sender.SDK
	sender.mu.Unlock()

	if senderSDK == nil {
		return fmt.Errorf("sender SDK not connected")
	}

	receiver.mu.Lock()
	receiverAddr := receiver.SparkAddr
	receiver.mu.Unlock()

	// Create payment request using Spark address
	var payAmount sdk.PayAmount = sdk.PayAmountBitcoin{AmountSats: amountSats}
	prepareReq := sdk.PrepareSendPaymentRequest{
		PaymentRequest: receiverAddr,
		PayAmount:      &payAmount,
	}

	prepareResp, err := senderSDK.PrepareSendPayment(prepareReq)
	if err := unwrapSdkError(err); err != nil {
		return fmt.Errorf("prepare payment failed: %w", err)
	}

	// Send payment
	sendReq := sdk.SendPaymentRequest{
		PrepareResponse: prepareResp,
	}

	sendResp, err := senderSDK.SendPayment(sendReq)
	if err := unwrapSdkError(err); err != nil {
		return fmt.Errorf("send payment failed: %w", err)
	}

	fmt.Printf("[Payment %d] %s -> %s: %d sats via Spark (status: %v)\n",
		atomic.LoadInt64(&p.paymentCount)+1,
		sender.Name,
		receiver.Name,
		amountSats,
		sendResp.Payment.Status,
	)

	return nil
}

// sendLightningPayment sends a Lightning payment from sender to receiver.
func (p *PaymentLoop) sendLightningPayment(ctx context.Context, sender, receiver *SdkInstance, amountSats uint64) error {
	receiver.mu.Lock()
	receiverSDK := receiver.SDK
	receiver.mu.Unlock()

	if receiverSDK == nil {
		return fmt.Errorf("receiver SDK not connected")
	}

	sender.mu.Lock()
	senderSDK := sender.SDK
	sender.mu.Unlock()

	if senderSDK == nil {
		return fmt.Errorf("sender SDK not connected")
	}

	// Receiver creates a Bolt11 invoice
	receiveResp, err := receiverSDK.ReceivePayment(sdk.ReceivePaymentRequest{
		PaymentMethod: sdk.ReceivePaymentMethodBolt11Invoice{
			Description: "memtest payment",
			AmountSats:  &amountSats,
		},
	})
	if err := unwrapSdkError(err); err != nil {
		return fmt.Errorf("create invoice failed: %w", err)
	}

	invoice := receiveResp.PaymentRequest

	// Sender pays the invoice
	prepareReq := sdk.PrepareSendPaymentRequest{
		PaymentRequest: invoice,
	}

	prepareResp, err := senderSDK.PrepareSendPayment(prepareReq)
	if err := unwrapSdkError(err); err != nil {
		return fmt.Errorf("prepare payment failed: %w", err)
	}

	sendReq := sdk.SendPaymentRequest{
		PrepareResponse: prepareResp,
	}

	sendResp, err := senderSDK.SendPayment(sendReq)
	if err := unwrapSdkError(err); err != nil {
		return fmt.Errorf("send payment failed: %w", err)
	}

	fmt.Printf("[Payment %d] %s -> %s: %d sats via Lightning (status: %v)\n",
		atomic.LoadInt64(&p.paymentCount)+1,
		sender.Name,
		receiver.Name,
		amountSats,
		sendResp.Payment.Status,
	)

	return nil
}

// sendPayment sends a payment using the configured payment type.
func (p *PaymentLoop) sendPayment(ctx context.Context, sender, receiver *SdkInstance, amountSats uint64) error {
	switch p.cfg.PaymentType {
	case PaymentTypeSpark:
		return p.sendSparkPayment(ctx, sender, receiver, amountSats)
	case PaymentTypeLightning:
		return p.sendLightningPayment(ctx, sender, receiver, amountSats)
	case PaymentTypeBoth:
		// Alternate between Spark and Lightning based on payment count
		if atomic.LoadInt64(&p.paymentCount)%2 == 0 {
			return p.sendSparkPayment(ctx, sender, receiver, amountSats)
		}
		return p.sendLightningPayment(ctx, sender, receiver, amountSats)
	default:
		return p.sendSparkPayment(ctx, sender, receiver, amountSats)
	}
}

// checkAndRefundIfNeeded checks sender balance and funds from faucet if too low.
// Returns nil if sender has sufficient funds, error if funding fails.
func (p *PaymentLoop) checkAndRefundIfNeeded(ctx context.Context, sender *SdkInstance) error {
	// Minimum balance threshold - need enough for payment + fees
	minBalance := uint64(5000)

	balance, err := sender.GetBalance()
	if err != nil {
		return fmt.Errorf("failed to get balance: %w", err)
	}

	if balance >= minBalance {
		return nil
	}

	fmt.Printf("\n[Refunding] %s balance too low (%d sats), requesting funds from faucet...\n", sender.Name, balance)

	// Fund from faucet (max 50k sats)
	if err := p.faucet.EnsureFunded(ctx, sender.BitcoinAddr, 50000); err != nil {
		return fmt.Errorf("failed to fund %s: %w", sender.Name, err)
	}

	// Wait for funds to be confirmed
	targetBalance := uint64(10000)
	maxWait := 5 * time.Minute
	pollInterval := 5 * time.Second
	startWait := time.Now()

	for {
		newBalance, _ := sender.GetBalance()
		if newBalance >= targetBalance {
			fmt.Printf("[Refunding] %s funded: %d sats\n\n", sender.Name, newBalance)
			return nil
		}

		if time.Since(startWait) > maxWait {
			return fmt.Errorf("timeout waiting for %s funds: %d", sender.Name, newBalance)
		}

		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(pollInterval):
			fmt.Printf("[Refunding] Waiting for %s funds... %d sats\n", sender.Name, newBalance)
		}
	}
}

// reconnectCycle performs a disconnect/reconnect cycle.
func (p *PaymentLoop) reconnectCycle(ctx context.Context) error {
	// Clean up listener managers before disconnect
	if p.aliceListeners != nil {
		p.aliceListeners.RemoveAll()
	}
	if p.bobListeners != nil {
		p.bobListeners.RemoveAll()
	}

	if err := p.pair.Reconnect(ctx, p.cfg.AliceSeed, p.cfg.BobSeed); err != nil {
		return err
	}

	// Re-create listener managers
	if p.cfg.ListenerChurn {
		p.aliceListeners = NewListenerManager(p.pair.Alice.SDK)
		p.bobListeners = NewListenerManager(p.pair.Bob.SDK)
	}

	// Wait for sync after reconnect
	time.Sleep(5 * time.Second)

	return nil
}

// performListenerChurn adds and removes listeners.
func (p *PaymentLoop) performListenerChurn() {
	// Add 10 listeners
	p.aliceListeners.AddListeners(5)
	p.bobListeners.AddListeners(5)

	// Remove 10 listeners
	p.aliceListeners.RemoveListeners(5)
	p.bobListeners.RemoveListeners(5)
}
