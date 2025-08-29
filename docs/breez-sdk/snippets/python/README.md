## Steps to run locally
1. Build a python package
  - By running the publish-all-platforms CI in the breez-sdk repository (use dummy binaries)
  - or by downloading from Pypi
2. Download the wheel artifact for your platform
3. Unzip the artifact in the `snippets/python/packages` folder
4. Run `pip install packages/{NAME_OF_.WHL_FILE}`

To check the syntax:

```bash
cd snippets/python
python3 -m compileall src
```

To check the snippet against formatting and linter rules:

```bash
cd snippets/python/src
ruff check --ignore F841 --ignore F401 --add-noqa .
```

and fix all occurrences of the `# noqa` directive.
