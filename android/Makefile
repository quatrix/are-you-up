# Thin aliases over gradlew/adb so commands stay uniform with backend/
# and mac/.
APK := app/build/outputs/apk/debug/app-debug.apk

.PHONY: build test install uninstall run log clean

build:
	./gradlew --console=plain assembleDebug

test:
	./gradlew --console=plain test

install: build
	adb install -r $(APK)

uninstall:
	adb uninstall dev.areyouup

run: install
	adb shell am start -n dev.areyouup/.MainActivity

log:
	adb logcat -s are-you-up

clean:
	./gradlew --console=plain clean
