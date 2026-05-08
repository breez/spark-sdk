package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if no built-in
// PasskeyProvider ships for your target. Single API surface: DeriveSeeds
// for derivation, CreatePasskey for registration, IsSupported /
// CheckDomainAssociation for diagnostics. Single-salt derivation is the
// trivial 1-element bulk case.
type CustomPrfProvider struct{}

func (p *CustomPrfProvider) DeriveSeeds(salts []string) ([][]byte, error) {
	// Call platform passkey API with PRF extension. Use the dual-salt
	// ceremony when the authenticator supports it (one OS prompt for
	// N salts) and fall back to per-salt assertions otherwise.
	// Returns one 32-byte PRF output per salt in input order.
	panic("Implement using WebAuthn or native passkey APIs")
}

func (p *CustomPrfProvider) IsSupported() (bool, error) {
	// Check if a PRF-capable authenticator is reachable from this
	// platform / device.
	panic("Check platform passkey availability")
}

func (p *CustomPrfProvider) CreatePasskey(request breez_sdk_spark.CreatePasskeyRequest) (breez_sdk_spark.RegisteredCredential, error) {
	// Register a new credential and return its ID + AAGUID + BE flag.
	panic("Implement registration via native passkey API")
}

func (p *CustomPrfProvider) CheckDomainAssociation() (breez_sdk_spark.DomainAssociation, error) {
	// Optional: verify the app's identity against the platform's domain
	// verification source (e.g., Apple AASA CDN, Google Digital Asset
	// Links). Custom providers without a verification source return
	// Skipped, which tells callers "proceed with WebAuthn as normal".
	return breez_sdk_spark.DomainAssociationSkipped{
		Reason: "CustomPrfProvider does not verify domain association",
	}, nil
}

// ANCHOR_END: implement-prf-provider

func CheckAvailability() {
	// ANCHOR: check-availability
	prfProvider := &CustomPrfProvider{}

	available, err := prfProvider.IsSupported()
	if err == nil && available {
		// Show passkey as primary option
	} else {
		// Fall back to mnemonic flow
	}
	// ANCHOR_END: check-availability
}

func ConnectWithPasskey() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: connect-with-passkey
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil)

	// SignIn derives the wallet seed for an existing credential. With
	// bulk PRF on iOS+Android this is a single OS prompt that derives
	// master + label seeds in one ceremony.
	label := "personal"
	response, err := passkey.SignIn(breez_sdk_spark.SignInRequest{
		Label:      &label,
		ExtraSalts: []breez_sdk_spark.NamedSalt{},
	})
	if err != nil {
		return nil, err
	}

	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	sdk, err := breez_sdk_spark.Connect(breez_sdk_spark.ConnectRequest{
		Config:     config,
		Seed:       response.Wallet.Seed,
		StorageDir: "./.data",
	})
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: connect-with-passkey
	return sdk, nil
}

func RegisterNewPasskey() (*breez_sdk_spark.BreezSdk, error) {
	// ANCHOR: register-passkey
	// For a brand-new user with no existing passkey: Register() creates
	// the credential AND derives the wallet seed in one orchestrated
	// call. On iOS+Android this is 2 OS prompts total (1 create + 1
	// dual-salt assert) thanks to the SDK's bulk-PRF setup_wallet path.
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil)

	label := "personal"
	response, err := passkey.Register(breez_sdk_spark.RegisterRequest{
		Label:                &label,
		ExtraSalts:           []breez_sdk_spark.NamedSalt{},
		ExcludeCredentialIds: [][]byte{},
	})
	if err != nil {
		return nil, err
	}

	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	sdk, err := breez_sdk_spark.Connect(breez_sdk_spark.ConnectRequest{
		Config:     config,
		Seed:       response.Wallet.Seed,
		StorageDir: "./.data",
	})
	if err != nil {
		return nil, err
	}
	// ANCHOR_END: register-passkey
	return sdk, nil
}

func ListLabels() ([]string, error) {
	// ANCHOR: list-labels
	prfProvider := &CustomPrfProvider{}
	breezApiKey := "<breez api key>"
	relayConfig := &breez_sdk_spark.NostrRelayConfig{
		BreezApiKey: &breezApiKey,
	}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, relayConfig)

	// SignIn with no label runs in discovery mode: it derives the master
	// seed AND lists labels in the same ceremony, so a follow-up
	// ListLabels() reads from the cached identity for free.
	labels, err := passkey.ListLabels()
	if err != nil {
		return nil, err
	}

	for _, label := range labels {
		log.Printf("Found label: %s", label)
	}
	// ANCHOR_END: list-labels
	return labels, nil
}

func StoreLabel() error {
	// ANCHOR: store-label
	prfProvider := &CustomPrfProvider{}
	breezApiKey := "<breez api key>"
	relayConfig := &breez_sdk_spark.NostrRelayConfig{
		BreezApiKey: &breezApiKey,
	}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, relayConfig)

	// For a new label on an existing identity, call SignIn(newLabel)
	// first to seed the SDK's identity cache via setup_wallet, THEN
	// StoreLabel uses the cached identity for free (1 OS prompt total).
	err := passkey.StoreLabel("personal")
	if err != nil {
		return err
	}
	// ANCHOR_END: store-label
	return nil
}
