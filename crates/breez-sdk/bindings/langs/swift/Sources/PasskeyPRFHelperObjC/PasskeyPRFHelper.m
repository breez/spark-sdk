#import "include/PasskeyPRFHelper.h"

API_AVAILABLE(ios(18.0), macos(15.0))
@implementation PasskeyPRFHelper

+ (void)setAssertionPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialAssertionRequest *)request
                        withSalt:(NSData *)salt {
    ASAuthorizationPublicKeyCredentialPRFAssertionInputValues *values =
        [[ASAuthorizationPublicKeyCredentialPRFAssertionInputValues alloc] initWithSaltInput1:salt saltInput2:nil];
    ASAuthorizationPublicKeyCredentialPRFAssertionInput *input =
        [[ASAuthorizationPublicKeyCredentialPRFAssertionInput alloc] initWithInputValues:values perCredentialInputValues:nil];
    request.prf = input;
}

+ (void)setRegistrationPRFOnRequest:(ASAuthorizationPlatformPublicKeyCredentialRegistrationRequest *)request {
    ASAuthorizationPublicKeyCredentialPRFRegistrationInput *input =
        [[ASAuthorizationPublicKeyCredentialPRFRegistrationInput alloc] initWithInputValues:nil];
    request.prf = input;
}

+ (NSData * _Nullable)extractPRFOutputFromAssertion:(ASAuthorizationPlatformPublicKeyCredentialAssertion *)credential {
    ASAuthorizationPublicKeyCredentialPRFAssertionOutput *output = credential.prf;
    if (output == nil) {
        return nil;
    }
    return output.first;
}

@end
