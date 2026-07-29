#import "PasskeyPRFHelper.h"

API_AVAILABLE(ios(18.0), macos(15.0))
@implementation PasskeyPRFHelper

+ (void)setAssertionPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialAssertionRequest *)request
                        withSalt:(NSData *)salt {
    [self setAssertionPRFOnRequest:request withSalt1:salt salt2:nil];
}

+ (void)setAssertionPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialAssertionRequest *)request
                       withSalt1:(NSData *)salt1
                           salt2:(NSData * _Nullable)salt2 {
    ASAuthorizationPublicKeyCredentialPRFAssertionInputValues *values =
        [[ASAuthorizationPublicKeyCredentialPRFAssertionInputValues alloc] initWithSaltInput1:salt1 saltInput2:salt2];
    ASAuthorizationPublicKeyCredentialPRFAssertionInput *input =
        [[ASAuthorizationPublicKeyCredentialPRFAssertionInput alloc] initWithInputValues:values perCredentialInputValues:nil];
    request.prf = input;
}

+ (void)setRegistrationPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialRegistrationRequest *)request {
    [self setRegistrationPRFOnRequest:request withSalt1:nil salt2:nil];
}

+ (void)setRegistrationPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialRegistrationRequest *)request
                          withSalt1:(NSData * _Nullable)salt1
                              salt2:(NSData * _Nullable)salt2 {
    // nil salt1 asks only whether PRF is supported. Non-nil asks the
    // create ceremony to evaluate it too, which removes the assertion
    // that would otherwise follow.
    ASAuthorizationPublicKeyCredentialPRFAssertionInputValues *values = nil;
    if (salt1 != nil) {
        values = [[ASAuthorizationPublicKeyCredentialPRFAssertionInputValues alloc]
                     initWithSaltInput1:salt1 saltInput2:salt2];
    }
    ASAuthorizationPublicKeyCredentialPRFRegistrationInput *input =
        [[ASAuthorizationPublicKeyCredentialPRFRegistrationInput alloc] initWithInputValues:values];
    request.prf = input;
}

+ (NSData * _Nullable)extractPRFOutputFromRegistration:(ASAuthorizationPlatformPublicKeyCredentialRegistration *)credential {
    ASAuthorizationPublicKeyCredentialPRFRegistrationOutput *output = credential.prf;
    if (output == nil) {
        return nil;
    }
    return output.first;
}

+ (NSData * _Nullable)extractSecondPRFOutputFromRegistration:(ASAuthorizationPlatformPublicKeyCredentialRegistration *)credential {
    ASAuthorizationPublicKeyCredentialPRFRegistrationOutput *output = credential.prf;
    if (output == nil) {
        return nil;
    }
    return output.second;
}

+ (NSData * _Nullable)extractPRFOutputFromAssertion:(ASAuthorizationPlatformPublicKeyCredentialAssertion *)credential {
    ASAuthorizationPublicKeyCredentialPRFAssertionOutput *output = credential.prf;
    if (output == nil) {
        return nil;
    }
    return output.first;
}

+ (NSData * _Nullable)extractSecondPRFOutputFromAssertion:(ASAuthorizationPlatformPublicKeyCredentialAssertion *)credential {
    ASAuthorizationPublicKeyCredentialPRFAssertionOutput *output = credential.prf;
    if (output == nil) {
        return nil;
    }
    return output.second;
}

@end
