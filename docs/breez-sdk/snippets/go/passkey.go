package example

import (
	"errors"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

// ANCHOR: implement-prf-provider
// Implement the PrfProvider interface for a custom authenticator (hardware
// key, FIDO2, file-backed). Only DeriveSeeds and IsSupported are required.
type CustomPrfProvider struct{}

func (p *CustomPrfProvider) DeriveSeeds(request breez_sdk_spark.DeriveSeedsRequest) (breez_sdk_spark.DeriveSeedsOutput, error) {
	// Return one 32-byte PRF output per salt, in input order.
	panic("Implement using WebAuthn or native passkey APIs")
}

func (p *CustomPrfProvider) IsSupported() (bool, error) {
	panic("Check platform passkey availability")
}

func (p *CustomPrfProvider) CreatePasskey(excludeCredentials [][]byte) (breez_sdk_spark.PasskeyCredential, error) {
	// Register a credential and return its ID plus attestation.
	panic("Implement registration via native passkey API")
}

func (p *CustomPrfProvider) CheckDomainAssociation() (breez_sdk_spark.DomainAssociation, error) {
	return breez_sdk_spark.DomainAssociationSkipped{
		Reason: "CustomPrfProvider does not verify domain association",
	}, nil
}

// ANCHOR_END: implement-prf-provider

func CheckAvailability() {
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	// ANCHOR: check-availability
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

func SetupPasskeyClient() *breez_sdk_spark.PasskeyClient {
	// ANCHOR: setup-client
	prfProvider := &CustomPrfProvider{}
	apiKey := "<breez api key>"
	return breez_sdk_spark.NewPasskeyClient(prfProvider, &apiKey, nil)
	// ANCHOR_END: setup-client
}

func ConnectWithPasskey() (*breez_sdk_spark.BreezSdk, error) {
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	// ANCHOR: connect-with-passkey
	// Silent sign-in for a returning user, fall-through to register on a fresh device.
	label := "personal"
	response, err := passkey.ConnectWithPasskey(breez_sdk_spark.ConnectWithPasskeyRequest{
		Label: &label,
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
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	// ANCHOR: register-passkey
	label := "personal"
	response, err := passkey.Register(breez_sdk_spark.RegisterRequest{Label: &label})
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

func CredentialMetadata() error {
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	// ANCHOR: credential-metadata
	label := "personal"
	response, err := passkey.Register(breez_sdk_spark.RegisterRequest{Label: &label})
	if err != nil {
		return err
	}

	if response.Credential != nil {
		log.Println(response.Credential.CredentialId)   // Persist to reopen the same wallet on sign-in
		log.Println(response.Credential.Aaguid)         // Authenticator model (display hint, unverified)
		log.Println(response.Credential.BackupEligible) // Whether the passkey syncs across devices
	}

	// Pin the stored credential ID so the OS can't substitute a sibling
	// credential, which would derive a different wallet.
	signInResponse, err := passkey.SignIn(breez_sdk_spark.SignInRequest{
		Label:            &label,
		AllowCredentials: &[][]byte{
			// stored CredentialId bytes
		},
	})
	if err != nil {
		return err
	}
	log.Println(signInResponse.Wallet.Seed)  // Pass to connect() to open the wallet
	log.Println(signInResponse.Wallet.Label) // Label this wallet was derived from
	log.Println(signInResponse.Labels)       // This passkey's labels (populated on discovery sign-in)
	log.Println(signInResponse.Credential)   // Credential signed in with (credential_id only)
	// ANCHOR_END: credential-metadata
	return nil
}

func ListLabels() ([]string, error) {
	prfProvider := &CustomPrfProvider{}
	breezApiKey := "<breez api key>"
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, &breezApiKey, nil)
	// ANCHOR: list-labels
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
	prfProvider := &CustomPrfProvider{}
	breezApiKey := "<breez api key>"
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, &breezApiKey, nil)
	// ANCHOR: store-label
	err := passkey.Labels().Store("personal")
	if err != nil {
		return err
	}
	// ANCHOR_END: store-label
	return nil
}

func CheckDomain() error {
	// ANCHOR: domain-association
	// Diagnostic only: never blocks the ceremony.
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
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	// ANCHOR: recover-already-exists
	label := "personal"
	registerResponse, err := passkey.Register(breez_sdk_spark.RegisterRequest{
		Label: &label,
		ExcludeCredentials: &[][]byte{
			// app-persisted credential IDs from prior registrations
		},
	})
	if err == nil {
		return &registerResponse.Wallet, nil
	}

	if !errors.Is(err, breez_sdk_spark.ErrPrfProviderErrorCredentialAlreadyExists) {
		return nil, err
	}

	// A matching credential already exists; sign in to it instead.
	signInResponse, err := passkey.SignIn(breez_sdk_spark.SignInRequest{Label: &label})
	if err != nil {
		return nil, err
	}
	return &signInResponse.Wallet, nil
	// ANCHOR_END: recover-already-exists
}

func HandleTimeout() (*breez_sdk_spark.SignInResponse, error) {
	prfProvider := &CustomPrfProvider{}
	passkey := breez_sdk_spark.NewPasskeyClient(prfProvider, nil, nil)

	// ANCHOR: handle-timeout
	// Biometric inactivity timeout, distinct from a user cancel.
	label := "personal"
	response, err := passkey.SignIn(breez_sdk_spark.SignInRequest{Label: &label})
	if err != nil {
		if errors.Is(err, breez_sdk_spark.ErrPrfProviderErrorUserTimedOut) {
			// Show a retry UI. Do NOT auto-retry without user input.
			log.Print("Sign-in timed out: show \"Try Again\" UI.")
		}
		return nil, err
	}
	return &response, nil
	// ANCHOR_END: handle-timeout
}
