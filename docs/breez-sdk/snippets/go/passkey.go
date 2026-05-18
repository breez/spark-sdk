package example

import (
	"errors"
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
	// Register a new credential and return its ID, the WebAuthn user.id
	// the platform recorded (returned for host-side correlation, never
	// host-supplied), AAGUID, and BE flag.
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
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil)

	// CheckAvailability collapses IsSupported + CheckDomainAssociation
	// into a single tagged value. Branch on the variant the host needs.
	availability, err := passkey.CheckAvailability()
	if err != nil {
		return
	}
	switch r := availability.(type) {
	case breez_sdk_spark.PasskeyAvailabilityAvailable:
		_ = r // Show passkey as primary option.
	case breez_sdk_spark.PasskeyAvailabilityPrfUnsupported:
		_ = r // Fall back to mnemonic flow.
	case breez_sdk_spark.PasskeyAvailabilityNotAssociated:
		log.Printf("Domain association failed (source=%s): %s", r.Source, r.Reason)
	case breez_sdk_spark.PasskeyAvailabilitySkipped:
		_ = r // No verification source on this platform; proceed normally.
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

	// Hosts SHOULD persist Credential.CredentialId (for ExcludeCredentialIds
	// bookkeeping) and Credential.UserId (for server-side correlation).
	// The SDK generates UserId; it is never host-supplied.
	_ = response.Credential.CredentialId
	_ = response.Credential.UserId

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
	defaultLabel := "personal"
	config := &breez_sdk_spark.PasskeyConfig{
		BreezApiKey: &breezApiKey,
		// Optional: override the default wallet label used when Register /
		// SignIn receive `Label = nil`. Falls back to the SDK's internal
		// "Default" when unset.
		DefaultLabel: &defaultLabel,
	}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, config)

	// SignIn with no label runs in discovery mode: it derives the master
	// seed AND lists labels in the same ceremony, so a follow-up
	// Labels().List() reads from the cached identity for free.
	labels, err := passkey.Labels().List()
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
	config := &breez_sdk_spark.PasskeyConfig{
		BreezApiKey: &breezApiKey,
	}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, config)

	// For a new label on an existing identity, call SignIn(newLabel)
	// first to seed the SDK's identity cache via setup_wallet, THEN
	// Labels().Store() uses the cached identity for free (1 OS prompt total).
	err := passkey.Labels().Store("personal")
	if err != nil {
		return err
	}
	// ANCHOR_END: store-label
	return nil
}

func SingleCtaOnboarding() (*breez_sdk_spark.Wallet, error) {
	// ANCHOR: signin-fallback-register
	// Single-CTA onboarding: try silent SignIn first, fall through to
	// Register on CredentialNotFound. The OS shows ONE prompt for a
	// returning user (silent assertion succeeds), TWO for a new user
	// (silent assertion fast-fails, then create + dual-salt assert).
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil)

	// Discovery mode (Label=nil): derives master + configured default
	// label in a single ceremony. The fresh-device user fast-fails in
	// <300ms with no UI shown.
	response, err := passkey.SignIn(breez_sdk_spark.SignInRequest{
		Label:      nil,
		ExtraSalts: []breez_sdk_spark.NamedSalt{},
	})
	if err == nil {
		return &response.Wallet, nil
	}

	// CredentialNotFound is the SDK's classification for "no matching
	// credential on this device", including iOS's <300ms fast-fail case
	// where the platform conflates no-cred with user-cancel. The error
	// now carries a string payload with diagnostic detail.
	if !errors.Is(err, breez_sdk_spark.ErrPrfProviderErrorCredentialNotFound) {
		return nil, err
	}

	label := "personal"
	registerResponse, err := passkey.Register(breez_sdk_spark.RegisterRequest{
		Label:                &label,
		ExtraSalts:           []breez_sdk_spark.NamedSalt{},
		ExcludeCredentialIds: [][]byte{},
	})
	if err != nil {
		return nil, err
	}
	return &registerResponse.Wallet, nil
	// ANCHOR_END: signin-fallback-register
}

func CheckDomain() error {
	// ANCHOR: domain-association
	// Verify Apple AASA / Android Asset Links / Web Related Origins
	// before the first WebAuthn ceremony. Diagnostic only: never blocks.
	prfProvider := &CustomPrfProvider{}
	result, err := prfProvider.CheckDomainAssociation()
	if err != nil {
		return err
	}

	switch r := result.(type) {
	case breez_sdk_spark.DomainAssociationAssociated:
		// Safe to proceed.
		_ = r
	case breez_sdk_spark.DomainAssociationNotAssociated:
		// Configuration is wrong (entitlement missing, AASA stale,
		// assetlinks malformed). Surface a developer-facing error.
		log.Printf("Domain association failed (source=%s): %s", r.Source, r.Reason)
		return nil
	case breez_sdk_spark.DomainAssociationSkipped:
		// Verification could not be performed (offline, endpoint
		// timeout, no public-suffix match). Proceed normally: this is
		// NOT a negative signal.
		_ = r
	}
	// ANCHOR_END: domain-association
	return nil
}

func RecoverFromAlreadyExists() (*breez_sdk_spark.Wallet, error) {
	// ANCHOR: recover-already-exists
	// The OS rejected Register because the user's password manager
	// already holds a credential matching `ExcludeCredentialIds`.
	// Route the user to the sign-in path: the OS picker will surface
	// the existing credential and the SDK's identity cache will warm
	// up on the assertion.
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil)

	label := "personal"
	registerResponse, err := passkey.Register(breez_sdk_spark.RegisterRequest{
		Label:      &label,
		ExtraSalts: []breez_sdk_spark.NamedSalt{},
		ExcludeCredentialIds: [][]byte{
			// app-persisted credential IDs from prior registrations
		},
	})
	if err == nil {
		return &registerResponse.Wallet, nil
	}

	if !errors.Is(err, breez_sdk_spark.ErrPrfProviderErrorCredentialAlreadyExists) {
		return nil, err
	}

	// Flip to sign-in. The existing credential's PRF output is the
	// same wallet seed the host would have minted on register.
	signInResponse, err := passkey.SignIn(breez_sdk_spark.SignInRequest{
		Label:      &label,
		ExtraSalts: []breez_sdk_spark.NamedSalt{},
	})
	if err != nil {
		return nil, err
	}
	return &signInResponse.Wallet, nil
	// ANCHOR_END: recover-already-exists
}

func HandleTimeout() (*breez_sdk_spark.SignInResponse, error) {
	// ANCHOR: handle-timeout
	// The OS biometric inactivity timeout (~55s+) tore down the prompt
	// without user intent. Distinct from a real cancel: hosts may
	// surface a re-prompt UI without treating it as the user opting
	// out. The SDK fires PrfProviderErrorUserTimedOut when assertion or
	// register elapsed time crosses 55_000 ms.
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil)

	label := "personal"
	response, err := passkey.SignIn(breez_sdk_spark.SignInRequest{
		Label:      &label,
		ExtraSalts: []breez_sdk_spark.NamedSalt{},
	})
	if err != nil {
		if errors.Is(err, breez_sdk_spark.ErrPrfProviderErrorUserTimedOut) {
			// Show a sticky retry screen with timeout-specific copy.
			// Do NOT auto-retry without user input.
			log.Print("Sign-in timed out: show \"Try Again\" UI.")
		}
		return nil, err
	}
	return &response, nil
	// ANCHOR_END: handle-timeout
}
