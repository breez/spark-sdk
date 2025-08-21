#!/usr/bin/env python

from setuptools import setup

LONG_DESCRIPTION = """# Breez Spark SDK
Python language bindings for the [Breez Spark SDK](https://github.com/breez/spark-sdk).

## Installing

```shell
pip install breez_sdk_spark
```
"""

setup(
    name="breez_sdk_spark",
    version="0.2.7.dev9",
    description="Python language bindings for the Breez Spark SDK",
    long_description=LONG_DESCRIPTION,
    long_description_content_type="text/markdown",
    packages=["breez_sdk_spark"],
    package_dir={"breez_sdk_spark": "./src/breez_sdk_spark"},
    include_package_data=True,
    package_data={"breez_sdk_spark": ["*.dylib", "*.so", "*.dll"]},
    url="https://github.com/breez/spark-sdk",
    author="Breez <contact@breez.technology>",
    license="MIT",
    has_ext_modules=lambda: True,
)
