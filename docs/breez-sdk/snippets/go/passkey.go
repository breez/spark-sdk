package example

import (
	"errors"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for custom logic if no built-in
// PasskeyProvider ships for your target. Three required methods:
// DeriveSeeds for derivation, IsSupported for the capability probe;
// CreatePasskey for registration is optional.
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

func (p *CustomPrfProvider) CreatePasskey(excludeCredentialIds [][]byte) (breez_sdk_spark.RegisteredCredential, error) {
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
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

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
	// Single-CTA onboarding: silent sign-in for a returning user,
	// fall-through to register on a fresh device. Internally pins
	// `PreferImmediatelyAvailableCredentials = true` so the silent
	// attempt fast-fails (no UI) when no local credential exists; only
	// `CredentialNotFound` flips to register, all other errors (cancel
	// / timeout / configuration) propagate unchanged.
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	label := "personal"
	response, err := passkey.ConnectWithPasskey(breez_sdk_spark.ConnectWithPasskeyRequest{
		Label: &label,
	})
	if err != nil {
		return nil, err
	}

	// RegisteredCredential doubles as the path discriminator: non-nil
	// when a new credential was just registered (persist CredentialId
	// for future ExcludeCredentialIds); nil when silent sign-in
	// succeeded for an existing credential.
	if response.RegisteredCredential != nil {
		_ = response.RegisteredCredential.CredentialId
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
	// the credential AND derives the seed in one orchestrated
	// call. On iOS+Android this is 2 OS prompts total (1 create + 1
	// dual-salt assert) thanks to the SDK's bulk-PRF path.
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	label := "personal"
	response, err := passkey.Register(breez_sdk_spark.RegisterRequest{Label: &label})
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
	defaultLabel := "personal"
	config := &breez_sdk_spark.PasskeyConfig{
		// Optional: override the default label used when Register /
		// SignIn receive `Label = nil`. Falls back to the SDK's internal
		// "Default" when unset.
		DefaultLabel: &defaultLabel,
	}
	// breezApiKey enables authenticated (NIP-42) Breez relay access
	// for label sync; pass nil for public-relay-only.
	breezApiKey := "<breez api key>"
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, &breezApiKey, config)

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
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, &breezApiKey, nil)

	// For a new label on an existing identity, call SignIn(newLabel)
	// first to warm the SDK's identity cache, THEN
	// Labels().Store() uses the cached identity for free (1 OS prompt total).
	err := passkey.Labels().Store("personal")
	if err != nil {
		return err
	}
	// ANCHOR_END: store-label
	return nil
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
		_ = r // Safe to proceed.
	case breez_sdk_spark.DomainAssociationNotAssociated:
		log.Printf("Domain association failed (source=%s): %s", r.Source, r.Reason)
		return nil
	case breez_sdk_spark.DomainAssociationSkipped:
		_ = r // Verification could not be performed; proceed normally.
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
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	label := "personal"
	registerResponse, err := passkey.Register(breez_sdk_spark.RegisterRequest{
		Label: &label,
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

	signInResponse, err := passkey.SignIn(breez_sdk_spark.SignInRequest{Label: &label})
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
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	label := "personal"
	response, err := passkey.SignIn(breez_sdk_spark.SignInRequest{Label: &label})
	if err != nil {
		if errors.Is(err, breez_sdk_spark.ErrPrfProviderErrorUserTimedOut) {
			log.Print("Sign-in timed out: show \"Try Again\" UI.")
		}
		return nil, err
	}
	return &response, nil
	// ANCHOR_END: handle-timeout
}
