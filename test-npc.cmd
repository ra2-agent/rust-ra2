@echo off
node "%~dp0scripts\npc-test.mjs" %*
exit /b %errorlevel%
