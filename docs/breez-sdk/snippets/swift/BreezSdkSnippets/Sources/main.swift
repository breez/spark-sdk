// The Swift Programming Language
// https://docs.swift.org/swift-book

Task {
    do {
        let _ = try await initSdk()
    } catch {
        print(error.localizedDescription)
    }
}
