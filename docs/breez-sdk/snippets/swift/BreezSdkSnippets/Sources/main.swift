// The Swift Programming Language
// https://docs.swift.org/swift-book

Task {
    do {
        let sdk = try initSdk()
    } catch {
        print(error.localizedDescription)
    }
}
