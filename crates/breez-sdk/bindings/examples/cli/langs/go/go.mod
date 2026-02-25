module github.com/breez/spark-sdk/bindings/examples/cli/langs/go

go 1.19

require (
	github.com/breez/breez-sdk-spark-go v0.10.0
	github.com/chzyer/readline v1.5.1
	github.com/tyler-smith/go-bip39 v1.1.0
)

require (
	golang.org/x/crypto v0.0.0-20200622213623-75b288015ac9 // indirect
	golang.org/x/sys v0.0.0-20220310020820-b874c991c1a5 // indirect
)

// Uncomment to use locally generated bindings (run `make setup-local` first):
// replace github.com/breez/breez-sdk-spark-go => ../../../../ffi/golang
