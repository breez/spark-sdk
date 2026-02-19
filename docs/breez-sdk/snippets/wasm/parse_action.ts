import type { BreezSdk, ParsedAction, SendAction } from '@breeztech/breez-sdk-spark'
import { Breez } from '@breeztech/breez-sdk-spark'

// ANCHOR: parse-action
const parseActionExample = async (sdk: BreezSdk): Promise<void> => {
  const input = 'an input to be parsed...'

  const action = await sdk.parseAction(input)

  switch (action.type) {
    case 'send': {
      const sendAction = action as { type: 'send' } & SendAction
      console.log(`Send destination: ${sendAction.type}`)
      break
    }
    case 'receive':
      console.log('Can receive funds (e.g. LNURL-withdraw)')
      break
    case 'authenticate':
      console.log(`LNURL-Auth for domain: ${action.domain}`)
      await sdk.authenticate(action)
      break
    case 'multi':
      console.log(`BIP21 with ${action.actions.length} payment options`)
      break
    case 'unsupported':
      console.log(`Unsupported input: ${action.raw}`)
      break
  }
}
// ANCHOR_END: parse-action

// ANCHOR: parse-action-static
const parseActionStaticExample = async (): Promise<void> => {
  const input = 'lnbc100n1...'

  // Use Breez.parseAction() without an SDK instance
  const action = await Breez.parseAction(input)

  if (action.type === 'send') {
    console.log('Can send payment')
  }
}
// ANCHOR_END: parse-action-static

export { parseActionExample, parseActionStaticExample }
