const { Command, Option } = require('commander')
const { parse: parseShell } = require('shell-quote')
const { disconnect, getInfo, receivePayment, sendPayment, syncWallet, listPayments, getPayment } = require('./action.js')
const { prompt } = require('./prompt.js')

const initCommand = () => {
    const program = new Command()
    program.exitOverride()
    program.name('spark-wasm-cli').description('CLI for Breez Spark SDK Wasm')

    program.command('disconnect').alias('exit').description('Exit the CLI').action(disconnect)

    program.command('get-info').description('Get the balance and general info of the current instance').action(getInfo)

    program
        .command('receive-payment')
        .description('Receive a payment')
        .addOption(
            new Option('-m, --payment-method <choice>', 'The method to use when receiving')
                .makeOptionMandatory(true)
                .choices(['sparkAddress', 'bitcoinAddress', 'bolt11Invoice'])
        )
        .addOption(
            new Option('-d, --description <text>', 'Description for bolt11 invoice (required when payment-method is bolt11Invoice)')
        )
        .addOption(
            new Option('-a, --amount-sats <number>', 'Amount in satoshis for bolt11 invoice (optional)')
                .argParser(parseInt)
        )
        .action(receivePayment)

    program
        .command('send-payment')
        .description('Send a payment')
        .addOption(
            new Option('-p, --payment-request <text>', 'Payment request string').makeOptionMandatory(true)
        )
        .addOption(
            new Option('-a, --amount-sats <number>', 'Amount in satoshis for when the payment request doesn\'t specify it')
                .argParser(parseInt)
        )
        .action(sendPayment)

    program.command('sync-wallet').description('Sync the wallet').action(syncWallet)

    program
        .command('list-payments')
        .description('List payments')
        .addOption(
            new Option('-o, --offset <number>', 'Offset for pagination')
                .argParser(parseInt)
        )
        .addOption(
            new Option('-l, --limit <number>', 'Limit for pagination')
                .argParser(parseInt)
        )
        .action(listPayments)

    program
        .command('get-payment')
        .description('Get a payment')
        .addOption(
            new Option('-i, --payment-id <text>', 'Payment ID')
                .makeOptionMandatory(true)
        )
        .action(getPayment)

    return program
}


const main = () => {
    return new Promise(async (resolve) => {
        while (true) {
            try {
                const res = await prompt('sdk')
                if (res.trim().toLowerCase() === 'exit') {
                    disconnect()
                    resolve()
                    break
                } else {
                    const cmd = res.length > 0 ? res : '-h'
                    const program = initCommand()
                    await program.parseAsync(parseShell(cmd).map(entry => typeof entry === 'string' ? entry : String(entry)), { from: 'user' })
                }
            } catch (e) {
                if (!e.code) {
                    console.error('Error:', e)
                }
            }
        }
    })
}

main()