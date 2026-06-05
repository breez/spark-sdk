package main

import (
	"fmt"
	"sort"
	"strings"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

// WebhookCmd represents a single webhooks subcommand.
type WebhookCmd struct {
	Name        string
	Description string
	Run         func(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error
}

// WebhookCommandNames lists all webhooks subcommand names (used for REPL completion).
var WebhookCommandNames = []string{
	"webhooks register",
	"webhooks unregister",
	"webhooks list",
}

// BuildWebhookRegistry returns a map of webhooks subcommand name to WebhookCmd.
func BuildWebhookRegistry() map[string]WebhookCmd {
	return map[string]WebhookCmd{
		"register": {
			Name:        "register",
			Description: "Register a new webhook",
			Run:         handleWebhookRegister,
		},
		"unregister": {
			Name:        "unregister",
			Description: "Unregister a webhook",
			Run:         handleWebhookUnregister,
		},
		"list": {
			Name:        "list",
			Description: "List all registered webhooks",
			Run:         handleWebhookList,
		},
	}
}

// DispatchWebhookCommand dispatches a webhooks subcommand.
func DispatchWebhookCommand(args []string, sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance) {
	registry := BuildWebhookRegistry()

	if len(args) == 0 || args[0] == "help" {
		fmt.Println("\nWebhooks subcommands:")
		names := make([]string, 0, len(registry))
		for name := range registry {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			cmd := registry[name]
			fmt.Printf("  webhooks %-30s %s\n", name, cmd.Description)
		}
		fmt.Println()
		return
	}

	subName := args[0]
	subArgs := args[1:]

	cmd, ok := registry[subName]
	if !ok {
		fmt.Printf("Unknown webhooks subcommand: %s. Use 'webhooks help' for available commands.\n", subName)
		return
	}

	if err := cmd.Run(sdk, rl, subArgs); err != nil {
		fmt.Printf("Error: %v\n", err)
	}
}

// ---------------------------------------------------------------------------
// Webhooks command handlers
// ---------------------------------------------------------------------------

// --- register ---

func handleWebhookRegister(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 3 {
		fmt.Println("Usage: webhooks register <url> <secret> <event_type> [<event_type> ...]")
		fmt.Println("Event types: lightning-receive, lightning-send, coop-exit, static-deposit")
		return nil
	}

	url := args[0]
	secret := args[1]
	eventTypes, err := parseWebhookEventTypes(args[2:])
	if err != nil {
		return err
	}

	result, err := sdk.RegisterWebhook(breez_sdk_spark.RegisterWebhookRequest{
		Url:        url,
		Secret:     secret,
		EventTypes: eventTypes,
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- unregister ---

func handleWebhookUnregister(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: webhooks unregister <webhook_id>")
		return nil
	}

	if err := liftError(sdk.UnregisterWebhook(breez_sdk_spark.UnregisterWebhookRequest{
		WebhookId: args[0],
	})); err != nil {
		return err
	}
	fmt.Println("Webhook unregistered successfully")
	return nil
}

// --- list ---

func handleWebhookList(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, _ []string) error {
	result, err := sdk.ListWebhooks()
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

func parseWebhookEventTypes(values []string) ([]breez_sdk_spark.WebhookEventType, error) {
	var types []breez_sdk_spark.WebhookEventType
	for _, v := range values {
		switch strings.ToLower(v) {
		case "lightning-receive":
			types = append(types, breez_sdk_spark.WebhookEventTypeLightningReceiveFinished{})
		case "lightning-send":
			types = append(types, breez_sdk_spark.WebhookEventTypeLightningSendFinished{})
		case "coop-exit":
			types = append(types, breez_sdk_spark.WebhookEventTypeCoopExitFinished{})
		case "static-deposit":
			types = append(types, breez_sdk_spark.WebhookEventTypeStaticDepositFinished{})
		default:
			return nil, fmt.Errorf("unknown webhook event type: %s (valid: lightning-receive, lightning-send, coop-exit, static-deposit)", v)
		}
	}
	return types, nil
}
