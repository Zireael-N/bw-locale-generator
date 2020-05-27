#!/usr/bin/env python3

import sys
import yaml
import requests
from bs4 import BeautifulSoup

localized_names = {}


def output(module):
    for locale, mobs in localized_names.items():
        if locale == "enUS":
            print('local L = mod:GetLocale()')
        elif locale != "esES":
            print('local L = BigWigs:NewBossLocale("%s", "%s")' % (module, locale))
        else:
            print('local L = BigWigs:NewBossLocale("%s", "%s") or BigWigs:NewBossLocale("%s", "esMX")' % (module, locale, module))

        if locale != "enUS":
            print('if not L then return end')

        print('if L then')
        for shortname, localized_name in mobs.items():
            print('\tL.%s = "%s"' % (shortname, localized_name))
        print('end\n')
    return


def parse_page(locale, name, mobid):
    url = 'http://%s.wowhead.com/npc=%d' % (locale[0], mobid)
    try:
        response = requests.get(url)
        if 'notFound' in response.url:
            print('[%s] No result' % locale)
        else:
            soup = BeautifulSoup(response.content, 'html.parser')
            text = soup.find_all('h1', class_='heading-size-1')[0].get_text()
            localized_names[locale[1]][name] = text
    except:
        print('[%s] Error!' % locale)


def go(yamlfile, module):
    with open(yamlfile, 'r') as stream:
        try:
            data = yaml.safe_load(stream)
            locales = [['www', 'enUS'], ['de', 'deDE'], ['es', 'esES'], ['fr', 'frFR'], ['it', 'itIT'], ['pt', 'ptBR'], ['ru', 'ruRU'], ['ko', 'koKR'], ['cn', 'zhCN']]
            for locale in locales:
                localized_names[locale[1]] = {}
                for name, mobid in data.items():
                    parse_page(locale, name, mobid)
            output(module)
        except yaml.YAMLError as exc:
            print(exc)


if len(sys.argv) != 3:
    print("usage: ./%s moblist.yaml \"Module Name\"" % sys.argv[0])
    sys.exit()

go(sys.argv[1], sys.argv[2])
