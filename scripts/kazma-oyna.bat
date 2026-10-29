@echo off
chcp 65001 >nul
cd /d "%~dp0"
set /p OYUNCU=Oyuncu adi (3-16 harf, rakam veya _): 
kazma-cli.exe launch 1.21.1 "%OYUNCU%" --fabric --ram 4096
pause
