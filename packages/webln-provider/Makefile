default: build

build:
	npm install && npm run build

test:
	npm install && npm test

check: test

clean:
	rm -rf dist node_modules

# Individual package binding updates
update-flutter-binding:
	@echo "Updating Flutter WebLN binding..."
	@mkdir -p ../flutter/lib/src/webln
	@echo "// Auto-generated from packages/webln-provider - do not edit manually" > ../flutter/lib/src/webln/provider_script.dart
	@echo "// Run 'make update-webln-bindings' to regenerate" >> ../flutter/lib/src/webln/provider_script.dart
	@echo "" >> ../flutter/lib/src/webln/provider_script.dart
	@echo "/// WebLN provider script to inject into WebViews" >> ../flutter/lib/src/webln/provider_script.dart
	@echo "const String weblnProviderScript = r'''" >> ../flutter/lib/src/webln/provider_script.dart
	@cat dist/webln-provider.min.js >> ../flutter/lib/src/webln/provider_script.dart
	@echo "''';" >> ../flutter/lib/src/webln/provider_script.dart

update-react-native-binding:
	@echo "Updating React Native WebLN binding..."
	@mkdir -p ../react-native/src/webln
	@echo "// Auto-generated from packages/webln-provider - do not edit manually" > ../react-native/src/webln/providerScript.ts
	@echo "// Run 'make update-webln-bindings' to regenerate" >> ../react-native/src/webln/providerScript.ts
	@echo "" >> ../react-native/src/webln/providerScript.ts
	@echo "/** WebLN provider script to inject into WebViews */" >> ../react-native/src/webln/providerScript.ts
	@echo "export const weblnProviderScript = \`" >> ../react-native/src/webln/providerScript.ts
	@cat dist/webln-provider.min.js | sed 's/\\/\\\\/g' | sed 's/`/\\`/g' | sed 's/\$${/\\$${/g' >> ../react-native/src/webln/providerScript.ts
	@echo "\`;" >> ../react-native/src/webln/providerScript.ts

# Updates WebLN provider in Flutter and React Native packages
update-package-bindings: build update-flutter-binding update-react-native-binding
	@echo "Package bindings updated successfully"
