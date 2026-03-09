package main

import (
	"flag"
	"fmt"
	"sort"

	breez_sdk_spark "github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
	"github.com/chzyer/readline"
)

// ContactCmd represents a single contacts subcommand.
type ContactCmd struct {
	Name        string
	Description string
	Run         func(sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance, args []string) error
}

// ContactCommandNames lists all contacts subcommand names (used for REPL completion).
var ContactCommandNames = []string{
	"contacts add",
	"contacts update",
	"contacts delete",
	"contacts list",
}

// BuildContactRegistry returns a map of contacts subcommand name → ContactCmd.
func BuildContactRegistry() map[string]ContactCmd {
	return map[string]ContactCmd{
		"add": {
			Name:        "add",
			Description: "Add a new contact",
			Run:         handleContactAdd,
		},
		"update": {
			Name:        "update",
			Description: "Update an existing contact",
			Run:         handleContactUpdate,
		},
		"delete": {
			Name:        "delete",
			Description: "Delete a contact",
			Run:         handleContactDelete,
		},
		"list": {
			Name:        "list",
			Description: "List contacts",
			Run:         handleContactList,
		},
	}
}

// DispatchContactCommand dispatches a contacts subcommand.
func DispatchContactCommand(args []string, sdk *breez_sdk_spark.BreezSdk, rl *readline.Instance) {
	registry := BuildContactRegistry()

	if len(args) == 0 || args[0] == "help" {
		fmt.Println("\nContacts subcommands:")
		names := make([]string, 0, len(registry))
		for name := range registry {
			names = append(names, name)
		}
		sort.Strings(names)
		for _, name := range names {
			cmd := registry[name]
			fmt.Printf("  contacts %-30s %s\n", name, cmd.Description)
		}
		fmt.Println()
		return
	}

	subName := args[0]
	subArgs := args[1:]

	cmd, ok := registry[subName]
	if !ok {
		fmt.Printf("Unknown contacts subcommand: %s. Use 'contacts help' for available commands.\n", subName)
		return
	}

	if err := cmd.Run(sdk, rl, subArgs); err != nil {
		fmt.Printf("Error: %v\n", err)
	}
}

// ---------------------------------------------------------------------------
// Contacts command handlers
// ---------------------------------------------------------------------------

// --- add ---

func handleContactAdd(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 2 {
		fmt.Println("Usage: contacts add <name> <payment_identifier>")
		return nil
	}

	result, err := sdk.AddContact(breez_sdk_spark.AddContactRequest{
		Name:              args[0],
		PaymentIdentifier: args[1],
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- update ---

func handleContactUpdate(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 3 {
		fmt.Println("Usage: contacts update <id> <name> <payment_identifier>")
		return nil
	}

	result, err := sdk.UpdateContact(breez_sdk_spark.UpdateContactRequest{
		Id:                args[0],
		Name:              args[1],
		PaymentIdentifier: args[2],
	})
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}

// --- delete ---

func handleContactDelete(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	if len(args) < 1 {
		fmt.Println("Usage: contacts delete <id>")
		return nil
	}

	if err := liftError(sdk.DeleteContact(args[0])); err != nil {
		return err
	}
	fmt.Println("Contact deleted successfully")
	return nil
}

// --- list ---

func handleContactList(sdk *breez_sdk_spark.BreezSdk, _ *readline.Instance, args []string) error {
	fs := flag.NewFlagSet("contacts list", flag.ContinueOnError)
	offset := fs.Uint("offset", 0, "Number of contacts to skip")
	limit := fs.Uint("limit", 0, "Maximum number of contacts to return")
	if err := fs.Parse(args); err != nil {
		return err
	}

	req := breez_sdk_spark.ListContactsRequest{}
	if *offset > 0 {
		o := uint32(*offset)
		req.Offset = &o
	}
	if *limit > 0 {
		l := uint32(*limit)
		req.Limit = &l
	}

	result, err := sdk.ListContacts(req)
	if err = liftError(err); err != nil {
		return err
	}
	printValue(result)
	return nil
}
