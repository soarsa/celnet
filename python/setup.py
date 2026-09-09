from setuptools import setup, find_packages

setup(
    name="celnet",
    version="2026.9.0",
    packages=find_packages(),
    install_requires=[],
    extras_require={
        "dataframe": ["pandas", "numpy"],
        "websocket": ["websockets"],
    },
)
