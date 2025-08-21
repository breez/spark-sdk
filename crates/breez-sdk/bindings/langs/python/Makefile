.PHONY: init
init:
	make -C ../../ init

darwin:
	make -C ../../ python-darwin
	cp ../../ffi/python/breez_sdk_spark.py src/breez_sdk_spark
	cp ../../ffi/python/libbreez_sdk_spark_bindings.dylib src/breez_sdk_spark

linux:
	make -C ../../ python-linux
	cp ../../ffi/python/breez_sdk_spark.py src/breez_sdk_spark
	cp ../../ffi/python/libbreez_sdk_spark_bindings.so src/breez_sdk_spark
