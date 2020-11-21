# crAnki

A simple command-line tool for interacting with [Anki](https://apps.ankiweb.net/) database files. So-named because I started the project when I was annoyed by the lack of a text-backed flashcard system and the fact that the official Anki client for Windows appears to only be downloadable as a ~100MB installer.

## Basic usage
Currently crAnki only supports adding new cards:
```
cranki add <field1> <field2> <field3>...
```
The first time you run crAnki you should pass in arguments to specify where new entries should inserted. A configuration file will be written to disk so that these parameters need not be passed in every time (they will be read from the configuration file).
