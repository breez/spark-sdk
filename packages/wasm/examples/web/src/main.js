import './style.css'
import QRCode from 'qrcode'
import init, { initLogging, defaultConfig, SdkBuilder, parse } from '@breeztech/breez-sdk-spark/web'

// Configuration loaded from environment variables
const CONFIG = {
  mnemonic: import.meta.env.VITE_MNEMONIC || '',
  network: 'regtest',
  chainServiceUrl: 'https://regtest-mempool.loadtest.dev.sparkinfra.net/api',
  chainServiceUsername: import.meta.env.VITE_CHAIN_SERVICE_USERNAME || '',
  chainServicePassword: import.meta.env.VITE_CHAIN_SERVICE_PASSWORD || '',
}

class WebLogger {
  log = (logEntry) => {
    console.log(`[${logEntry.level}]: ${logEntry.line}`)
  }
}

class WebEventListener {
  onEvent = (event) => {
    console.log('SDK Event:', event)
    if (event === 'synced') {
      // Refresh all wallet data when synced
      console.log('Wallet synced - updating UI...')
      updateWalletInfo()
      loadPayments()

      // Show a brief notification to user
      showSyncNotification()
    }
  }
}

let sdk = null
let prepareReceiveResponse = null
let prepareSendResponse = null
let prepareLnurlResponse = null
let autoRefreshInterval = null

// UI Elements
const elements = {
  connectBtn: document.getElementById('connect-btn'),
  disconnectBtn: document.getElementById('disconnect-btn'),
  statusDot: document.getElementById('status-dot'),
  statusText: document.getElementById('status-text'),
  walletInfo: document.getElementById('wallet-info'),
  actions: document.getElementById('actions'),
  paymentsSection: document.getElementById('payments-section'),
  syncBtn: document.getElementById('sync-btn'),
  receiveBtn: document.getElementById('receive-btn'),
  sendBtn: document.getElementById('send-btn'),
  lnurlPayBtn: document.getElementById('lnurl-pay-btn'),
  
  // Modals
  receiveModal: document.getElementById('receive-modal'),
  sendModal: document.getElementById('send-modal'),
  lnurlPayModal: document.getElementById('lnurl-pay-modal'),
  loadingOverlay: document.getElementById('loading-overlay'),

  // Wallet info
  balance: document.getElementById('balance'),
  paymentsList: document.getElementById('payments-list'),

  // Receive modal
  paymentMethod: document.getElementById('payment-method'),
  description: document.getElementById('description'),
  amount: document.getElementById('amount'),
  bolt11Fields: document.getElementById('bolt11-fields'),
  prepareReceiveBtn: document.getElementById('prepare-receive-btn'),
  cancelReceiveBtn: document.getElementById('cancel-receive-btn'),
  receiveResult: document.getElementById('receive-result'),
  qrCode: document.getElementById('qr-code'),
  paymentRequest: document.getElementById('payment-request'),
  copyPaymentRequest: document.getElementById('copy-payment-request'),

  // Send modal
  paymentRequestInput: document.getElementById('payment-request-input'),
  sendAmount: document.getElementById('send-amount'),
  prepareSendBtn: document.getElementById('prepare-send-btn'),
  cancelSendBtn: document.getElementById('cancel-send-btn'),
  sendConfirmation: document.getElementById('send-confirmation'),
  confirmAmount: document.getElementById('confirm-amount'),
  confirmFee: document.getElementById('confirm-fee'),
  confirmSendBtn: document.getElementById('confirm-send-btn'),
  cancelConfirmBtn: document.getElementById('cancel-confirm-btn'),
  
  // LNURL Pay modal
  lnurlInput: document.getElementById('lnurl-input'),
  lnurlComment: document.getElementById('lnurl-comment'),
  lnurlAmountSection: document.getElementById('lnurl-amount-section'),
  lnurlAmount: document.getElementById('lnurl-amount'),
  lnurlAmountRange: document.getElementById('lnurl-amount-range'),
  prepareLnurlBtn: document.getElementById('prepare-lnurl-btn'),
  cancelLnurlBtn: document.getElementById('cancel-lnurl-btn'),
  lnurlConfirmation: document.getElementById('lnurl-confirmation'),
  lnurlConfirmAmount: document.getElementById('lnurl-confirm-amount'),
  lnurlConfirmFee: document.getElementById('lnurl-confirm-fee'),
  lnurlConfirmCommentSection: document.getElementById('lnurl-confirm-comment-section'),
  lnurlConfirmComment: document.getElementById('lnurl-confirm-comment'),
  confirmLnurlBtn: document.getElementById('confirm-lnurl-btn'),
  cancelLnurlConfirmBtn: document.getElementById('cancel-lnurl-confirm-btn'),
  
  loadingText: document.getElementById('loading-text'),
}

// Utility functions
function showLoading(text = 'Loading...') {
  elements.loadingText.textContent = text
  elements.loadingOverlay.style.display = 'flex'
}

function hideLoading() {
  elements.loadingOverlay.style.display = 'none'
}

function showModal(modal) {
  modal.style.display = 'flex'
}

function hideModal(modal) {
  modal.style.display = 'none'
}

function updateConnectionStatus(connected) {
  elements.statusDot.classList.toggle('connected', connected)
  elements.statusText.textContent = connected ? 'Connected' : 'Disconnected'
  elements.connectBtn.style.display = connected ? 'none' : 'inline-block'
  elements.disconnectBtn.style.display = connected ? 'inline-block' : 'none'
  elements.walletInfo.style.display = connected ? 'block' : 'none'
  elements.actions.style.display = connected ? 'block' : 'none'
  elements.paymentsSection.style.display = connected ? 'block' : 'none'
}

function showSyncNotification() {
  showNotification('✓ Wallet synced', '#646cff')
}

function showPaymentNotification(message, type = 'success') {
  const color = type === 'success' ? '#44ff44' : type === 'error' ? '#ff4444' : '#646cff'
  showNotification(message, color)
}

function showNotification(message, backgroundColor = '#646cff') {
  // Create a temporary notification
  const notification = document.createElement('div')
  notification.style.cssText = `
    position: fixed;
    top: 20px;
    right: 20px;
    background: ${backgroundColor};
    color: white;
    padding: 10px 15px;
    border-radius: 5px;
    box-shadow: 0 2px 10px rgba(0,0,0,0.3);
    z-index: 3000;
    font-size: 14px;
    transition: opacity 0.3s ease;
    max-width: 300px;
    word-wrap: break-word;
  `
  notification.textContent = message
  document.body.appendChild(notification)

  // Remove after 4 seconds
  setTimeout(() => {
    notification.style.opacity = '0'
    setTimeout(() => {
      if (notification.parentNode) {
        notification.parentNode.removeChild(notification)
      }
    }, 300)
  }, 4000)
}

async function initializeSdk() {
  try {
    showLoading('Initializing WASM...')

    // Initialize WASM
    await init()

    showLoading('Setting up SDK...')

    // Initialize logging
    const logger = new WebLogger()
    await initLogging(logger)

    // Check if we have required configuration
    if (!CONFIG.mnemonic) {
      throw new Error('Mnemonic is required')
    }

    // Get default config
    const config = defaultConfig(CONFIG.network)

    // Create SDK builder
    let sdkBuilder = SdkBuilder.new(config, CONFIG.mnemonic, './.data')

    // Add chain service if configured
    if (CONFIG.chainServiceUrl && CONFIG.chainServiceUsername && CONFIG.chainServicePassword) {
      sdkBuilder = sdkBuilder.withRestChainService(CONFIG.chainServiceUrl, {
        username: CONFIG.chainServiceUsername,
        password: CONFIG.chainServicePassword
      })
    }

    showLoading('Building SDK...')

    // Build the SDK
    sdk = await sdkBuilder.build()

    // Add event listener
    const eventListener = new WebEventListener()
    await sdk.addEventListener(eventListener)

    showLoading('Starting SDK...')

    updateConnectionStatus(true)
    await updateWalletInfo()
    await loadPayments()

    hideLoading()
  } catch (error) {
    hideLoading()
    console.error('Failed to initialize SDK:', error)
    alert(`Failed to initialize SDK: ${error.message}`)
  }
}

async function updateWalletInfo() {
  if (!sdk) return

  try {
    const info = await sdk.getInfo({})
    elements.balance.textContent = `${info.balanceSats.toLocaleString()} sats`
  } catch (error) {
    console.error('Failed to get wallet info:', error)
  }
}

async function loadPayments() {
  if (!sdk) return

  try {
    const response = await sdk.listPayments({ limit: 10 })
    displayPayments(response.payments)
  } catch (error) {
    console.error('Failed to load payments:', error)
  }
}

function displayPayments(payments) {
  elements.paymentsList.innerHTML = ''

  if (payments.length === 0) {
    elements.paymentsList.innerHTML = '<div style="padding: 1rem; text-align: center; color: #888;">No payments found</div>'
    return
  }

  payments.forEach(payment => {
    const paymentDiv = document.createElement('div')
    paymentDiv.className = 'payment-item'

    const direction = payment.paymentType === 'receive' ? 'Received' : 'Sent'
    const amount = payment.amount || 0

    // Extract description from payment details
    let description = 'No description'
    if (payment.details && payment.details.type === 'lightning' && payment.details.description) {
      description = payment.details.description
    }

    paymentDiv.innerHTML = `
      <div class="payment-info">
        <div><strong>${direction}: ${amount.toLocaleString()} sats</strong></div>
        <div style="font-size: 0.9em; color: #888;">${description}</div>
        <div style="font-size: 0.8em; color: #666;">${new Date(payment.timestamp * 1000).toLocaleString()}</div>
    </div>
      <div class="payment-status ${payment.status.toLowerCase()}">${payment.status}</div>
    `

    elements.paymentsList.appendChild(paymentDiv)
  })
}

async function syncWallet() {
  if (!sdk) return

  try {
    showLoading('Syncing wallet...')
    await sdk.syncWallet({})
    await updateWalletInfo()
    await loadPayments()
    hideLoading()
    showNotification('✓ Manual sync completed', '#646cff')
  } catch (error) {
    hideLoading()
    console.error('Failed to sync wallet:', error)
    showPaymentNotification(`Failed to sync wallet: ${error.message}`, 'error')
  }
}

async function prepareReceive() {
  try {
    const paymentMethodType = elements.paymentMethod.value
    const description = elements.description.value
    const amountStr = elements.amount.value
    const amountSats = amountStr ? parseInt(amountStr) : undefined

    let paymentMethod
    if (paymentMethodType === 'bolt11Invoice') {
      if (!description) {
        alert('Description is required for BOLT11 invoice')
        return
      }
      paymentMethod = {
        type: 'bolt11Invoice',
        description: description,
        amountSats: amountSats
      }
    } else {
      // For sparkAddress and bitcoinAddress, just pass the type
      paymentMethod = { type: paymentMethodType }
    }

    showLoading('Preparing receive payment...')

    prepareReceiveResponse = await sdk.prepareReceivePayment({ paymentMethod })

    const fees = prepareReceiveResponse.feeSats

    if (!confirm(`Fees: ${fees} sat. Are the fees acceptable?`)) {
      hideLoading()
      return
    }

    showLoading('Generating payment request...')

    const result = await sdk.receivePayment({ prepareResponse: prepareReceiveResponse })

    elements.paymentRequest.value = result.paymentRequest

    // Generate QR code
    elements.qrCode.innerHTML = ''
    const canvas = document.createElement('canvas')
    await QRCode.toCanvas(canvas, result.paymentRequest, { width: 256 })
    elements.qrCode.appendChild(canvas)

    elements.receiveResult.style.display = 'block'
    hideLoading()

    // Refresh wallet data after creating payment request
    setTimeout(async () => {
      await updateWalletInfo()
      await loadPayments()
    }, 1000)

  } catch (error) {
    hideLoading()
    console.error('Failed to prepare receive payment:', error)
    alert(`Failed to prepare receive payment: ${error.message}`)
  }
}

async function prepareSend() {
  try {
    const paymentRequest = elements.paymentRequestInput.value.trim()
    const amountStr = elements.sendAmount.value
    const amountSats = amountStr ? parseInt(amountStr) : undefined

    if (!paymentRequest) {
      alert('Payment request is required')
      return
    }

    showLoading('Preparing send payment...')

    prepareSendResponse = await sdk.prepareSendPayment({
      paymentRequest: paymentRequest,
      amountSats: amountSats
    })

    elements.confirmAmount.textContent = `${prepareSendResponse.amountSats.toLocaleString()} sats`
    elements.confirmFee.textContent = `${prepareSendResponse.feeSats.toLocaleString()} sats`

    elements.sendConfirmation.style.display = 'block'
    hideLoading()

  } catch (error) {
    hideLoading()
    console.error('Failed to prepare send payment:', error)
    alert(`Failed to prepare send payment: ${error.message}`)
  }
}

async function sendPayment() {
  try {
    showLoading('Sending payment...')

    const result = await sdk.sendPayment({ prepareResponse: prepareSendResponse })

    hideLoading()
    hideModal(elements.sendModal)

    // Show success notification
    showPaymentNotification('Payment sent successfully!', 'success')

    // Refresh wallet data immediately and then again after a delay
    await updateWalletInfo()
    await loadPayments()

    // Refresh again after a few seconds to catch any delayed updates
    setTimeout(async () => {
      await updateWalletInfo()
      await loadPayments()
    }, 3000)

  } catch (error) {
    hideLoading()
    console.error('Failed to send payment:', error)
    alert(`Failed to send payment: ${error.message}`)
  }
}

async function prepareLnurlPay() {
  try {
    const lnurlInput = elements.lnurlInput.value.trim()
    
    if (!lnurlInput) {
      alert('LNURL or Lightning Address is required')
      return
    }
    
    showLoading('Parsing LNURL...')
    
    // Parse the LNURL
    const input = await parse(lnurlInput)
    
    if (input.type !== 'lnurlPay') {
      hideLoading()
      alert('Invalid input: expected LNURL pay request or Lightning Address')
      return
    }
    
    // Show amount section with min/max range
    const minSendable = Math.ceil(input.minSendable / 1000)
    const maxSendable = Math.floor(input.maxSendable / 1000)
    
    elements.lnurlAmountRange.textContent = `Min: ${minSendable.toLocaleString()} sats, Max: ${maxSendable.toLocaleString()} sats`
    elements.lnurlAmountSection.style.display = 'block'
    elements.lnurlAmount.min = minSendable
    elements.lnurlAmount.max = maxSendable
    
    hideLoading()
    
    // Focus on amount input
    elements.lnurlAmount.focus()
    
    // Change the prepare button to continue with amount
    elements.prepareLnurlBtn.textContent = 'Continue'
    elements.prepareLnurlBtn.onclick = async () => {
      const amountStr = elements.lnurlAmount.value
      const amountSats = amountStr ? parseInt(amountStr) : undefined
      
      if (!amountSats || amountSats < minSendable || amountSats > maxSendable) {
        alert(`Please enter a valid amount between ${minSendable} and ${maxSendable} satoshis`)
        return
      }
      
      showLoading('Preparing LNURL payment...')
      
      try {
        const comment = elements.lnurlComment.value || null
        
        prepareLnurlResponse = await sdk.prepareLnurlPay({
          amountSats: amountSats,
          comment: comment,
          data: input,
          validateSuccessActionUrl: true
        })
        
        elements.lnurlConfirmAmount.textContent = `${prepareLnurlResponse.amountSats.toLocaleString()} sats`
        elements.lnurlConfirmFee.textContent = `${prepareLnurlResponse.feeSats.toLocaleString()} sats`
        
        if (comment) {
          elements.lnurlConfirmComment.textContent = comment
          elements.lnurlConfirmCommentSection.style.display = 'block'
        } else {
          elements.lnurlConfirmCommentSection.style.display = 'none'
        }
        
        elements.lnurlConfirmation.style.display = 'block'
        hideLoading()
        
      } catch (error) {
        hideLoading()
        console.error('Failed to prepare LNURL payment:', error)
        alert(`Failed to prepare LNURL payment: ${error.message}`)
      }
    }
    
  } catch (error) {
    hideLoading()
    console.error('Failed to parse LNURL:', error)
    alert(`Failed to parse LNURL: ${error.message}`)
  }
}

async function lnurlPay() {
  try {
    showLoading('Sending LNURL payment...')
    
    const result = await sdk.lnurlPay({ prepareResponse: prepareLnurlResponse })
    
    hideLoading()
    hideModal(elements.lnurlPayModal)
    
    // Show success notification
    showPaymentNotification('LNURL payment sent successfully!', 'success')
    
    // Refresh wallet data immediately and then again after a delay
    await updateWalletInfo()
    await loadPayments()
    
    // Refresh again after a few seconds to catch any delayed updates
    setTimeout(async () => {
      await updateWalletInfo()
      await loadPayments()
    }, 3000)
    
  } catch (error) {
    hideLoading()
    console.error('Failed to send LNURL payment:', error)
    alert(`Failed to send LNURL payment: ${error.message}`)
  }
}

async function disconnectSdk() {
  if (sdk) {
    try {
      await sdk.disconnect()
    } catch (error) {
      console.error('Error disconnecting SDK:', error)
    }
    sdk = null
  }
  updateConnectionStatus(false)
}

function copyToClipboard(text) {
  navigator.clipboard.writeText(text).then(() => {
    alert('Copied to clipboard!')
  }).catch(() => {
    alert('Failed to copy to clipboard')
  })
}

// Event listeners
elements.connectBtn.addEventListener('click', initializeSdk)
elements.disconnectBtn.addEventListener('click', disconnectSdk)
elements.syncBtn.addEventListener('click', syncWallet)

elements.receiveBtn.addEventListener('click', () => {
  elements.receiveResult.style.display = 'none'
  elements.sendConfirmation.style.display = 'none'
  elements.lnurlConfirmation.style.display = 'none'
  showModal(elements.receiveModal)
})

elements.sendBtn.addEventListener('click', () => {
  elements.receiveResult.style.display = 'none'
  elements.sendConfirmation.style.display = 'none'
  elements.lnurlConfirmation.style.display = 'none'
  showModal(elements.sendModal)
})

elements.lnurlPayBtn.addEventListener('click', () => {
  elements.receiveResult.style.display = 'none'
  elements.sendConfirmation.style.display = 'none'
  elements.lnurlConfirmation.style.display = 'none'
  elements.lnurlAmountSection.style.display = 'none'
  elements.prepareLnurlBtn.textContent = 'Prepare'
  elements.prepareLnurlBtn.onclick = null
  showModal(elements.lnurlPayModal)
})

elements.paymentMethod.addEventListener('change', () => {
  elements.bolt11Fields.style.display =
    elements.paymentMethod.value === 'bolt11Invoice' ? 'block' : 'none'
})

elements.prepareReceiveBtn.addEventListener('click', prepareReceive)
elements.cancelReceiveBtn.addEventListener('click', () => hideModal(elements.receiveModal))

elements.prepareSendBtn.addEventListener('click', prepareSend)
elements.cancelSendBtn.addEventListener('click', () => hideModal(elements.sendModal))

elements.confirmSendBtn.addEventListener('click', sendPayment)
elements.cancelConfirmBtn.addEventListener('click', () => {
  elements.sendConfirmation.style.display = 'none'
})

elements.prepareLnurlBtn.addEventListener('click', prepareLnurlPay)
elements.cancelLnurlBtn.addEventListener('click', () => hideModal(elements.lnurlPayModal))

elements.confirmLnurlBtn.addEventListener('click', lnurlPay)
elements.cancelLnurlConfirmBtn.addEventListener('click', () => {
  elements.lnurlConfirmation.style.display = 'none'
})

elements.copyPaymentRequest.addEventListener('click', () => {
  copyToClipboard(elements.paymentRequest.value)
})

// Close modals when clicking outside
elements.receiveModal.addEventListener('click', (e) => {
  if (e.target === elements.receiveModal) {
    hideModal(elements.receiveModal)
  }
})

elements.sendModal.addEventListener('click', (e) => {
  if (e.target === elements.sendModal) {
    hideModal(elements.sendModal)
  }
})

elements.lnurlPayModal.addEventListener('click', (e) => {
  if (e.target === elements.lnurlPayModal) {
    hideModal(elements.lnurlPayModal)
  }
})

// Initialize UI
updateConnectionStatus(false)
console.log('Breez SDK Spark Web Demo ready!')