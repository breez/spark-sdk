package example

import (
	"log"
	"math/big"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func PrepareSendPaymentReserveLeaves(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.PrepareSendPaymentResponse, error) {
	// ANCHOR: prepare-send-payment-reserve-leaves
	paymentRequest := "<payment request>"
	amountSats := new(big.Int).SetInt64(50_000)
	reserveLeaves := true

	request := breez_sdk_spark.PrepareSendPaymentRequest{
		PaymentRequest:    paymentRequest,
		Amount:            &amountSats,
		TokenIdentifier:   nil,
		ConversionOptions: nil,
		FeePolicy:         nil,
		ReserveLeaves:     &reserveLeaves,
	}
	response, err := sdk.PrepareSendPayment(request)
	if err != nil {
		return nil, err
	}

	// The reservation ID can be used to cancel the reservation if needed
	if response.ReservationId != nil {
		log.Printf("Reservation ID: %v", *response.ReservationId)
	}

	// Send payment as usual using the prepare response
	// sdk.SendPayment(breez_sdk_spark.SendPaymentRequest{PrepareResponse: response, Options: nil, IdempotencyKey: nil})
	// ANCHOR_END: prepare-send-payment-reserve-leaves
	return &response, nil
}

func CancelPrepareSendPayment(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: cancel-prepare-send-payment
	reservationId := "<reservation id from prepare response>"

	request := breez_sdk_spark.CancelPrepareSendPaymentRequest{
		ReservationId: reservationId,
	}
	err := sdk.CancelPrepareSendPayment(request)
	if err != nil {
		return err
	}
	// ANCHOR_END: cancel-prepare-send-payment
	return nil
}
