#!/usr/bin/env python3

import sys
import re
from collections import namedtuple

id_regexp = re.compile('^\s*(\d+),?\s*--\s*(.+)$')
variable_regexp = re.compile('^\s*L\.(\w+)\s*=\s*"(.+)"')

ids_start = re.compile('^mod:RegisterEnableMob\(')
variables_start = re.compile('^if L then')

ParseResult = namedtuple('ParseResult', 'variable_to_id missing_variables missing_ids')


def parse_lua(path):
    looking_at_ids = False
    looking_at_variables = False

    variables_dictionary = {}
    ids_dictionary = {}

    with open(path, encoding='utf-8') as file:
        for line in file:
            if looking_at_ids:
                matches = id_regexp.match(line)
                if matches:
                    ids_dictionary[matches[2].strip()] = matches[1]
                elif re.match('\)', line):
                    looking_at_ids = False
            elif looking_at_variables:
                matches = variable_regexp.match(line)
                if matches:
                    variables_dictionary[matches[2]] = matches[1]
                elif line == 'end':
                    looking_at_variables = False
            elif ids_start.match(line):
                looking_at_ids = True
            elif variables_start.match(line):
                looking_at_variables = True

    variable_to_id = {}
    missing_variables = []
    missing_ids = []

    for string, variable in variables_dictionary.items():
        if string in ids_dictionary:
            variable_to_id[variable] = ids_dictionary[string]
        else:
            missing_variables.append((variable, string))

    for string, mobId in ids_dictionary.items():
        if not string in variables_dictionary:
            missing_ids.append((mobId, string))

    return ParseResult(variable_to_id, missing_variables, missing_ids)


if __name__ == '__main__':
    if len(sys.argv) != 2:
        print('usage: ./%s module.lua' % sys.argv[0], file=sys.stderr)
        sys.exit()

    result = parse_lua(sys.argv[1])

    for variable, mobId in result.variable_to_id.items():
        print('%s: %s' % (variable, mobId))

    if len(result.missing_variables) > 0:
        print('\nMissing variables:', file=sys.stderr)
        for variable, string in result.missing_variables:
            print('%s ("%s")' % (variable, string), file=sys.stderr)

    if len(result.missing_ids) > 0:
        print('\nMissing IDs:', file=sys.stderr)
        for mobId, string in result.missing_ids:
            print('%s ("%s")' % (mobId, string), file=sys.stderr)
